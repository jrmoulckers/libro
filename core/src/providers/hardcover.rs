//! Hardcover connector.
//!
//! [Hardcover](https://hardcover.app/) is a social reading-tracker with an
//! **official public GraphQL API** at `https://api.hardcover.app/v1/graphql`,
//! authenticated with a user-supplied API key (generated on the user's Hardcover
//! account settings page: <https://hardcover.app/account/api>). Because this is
//! an official, documented API used with the user's own key against the user's
//! own data, it is a legitimate integration (see `ARCHITECTURE.md` → "Legal
//! boundaries"). With Goodreads' API retired, Hardcover is Libro's reading-
//! tracker path.
//!
//! Capabilities: [`ProviderCapabilities::PROGRESS_SYNC`] only — reading status,
//! ratings, shelves, and progress. Hardcover is **not** the user's
//! library-of-record, so it advertises neither `CATALOG` nor `HOLDS`. Its
//! [`Provider::list_library`] nonetheless returns the user's *tracked* shelves
//! (their own data) so aggregation can read them back.
//!
//! API reference: <https://docs.hardcover.app/api/getting-started/>
//!
//! Operations implemented:
//! * `me`                — validate the token, resolve the authenticated user id.
//! * `user_books` query  — read the user's shelves (status + rating) for read-back.
//! * `search` query      — resolve a [`Book`] to a Hardcover book id (Typesense).
//! * `insert_user_book`  — set/upsert reading status and/or rating.
//! * `update_user_book`  — update status/rating for an existing shelf entry.
//! * `insert_user_book_read` — record reading progress (pages/seconds).
//!
//! The Hardcover API is explicitly "in beta / in flux"; mutation input shapes may
//! change. Where the current docs are ambiguous, the closest documented form is
//! used and marked with a `TODO`.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::models::{Book, MediaType, Progress};
use crate::providers::{Provider, ProviderCapabilities, ProviderError, ProviderResult};

/// The official Hardcover GraphQL endpoint.
pub const HARDCOVER_GRAPHQL_ENDPOINT: &str = "https://api.hardcover.app/v1/graphql";

/// `User-Agent` sent with every request (the docs recommend identifying scripts).
const USER_AGENT: &str = concat!("Libro/", env!("CARGO_PKG_VERSION"), " (hardcover-connector)");

/// A Hardcover reading status.
///
/// Maps to the numeric `status_id` used throughout the Hardcover schema. See
/// <https://docs.hardcover.app/api/graphql/schemas/books#user-book-statuses>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadingStatus {
    WantToRead,
    CurrentlyReading,
    Read,
    /// Did Not Finish.
    Dnf,
}

impl ReadingStatus {
    /// The Hardcover `status_id` for this status.
    pub fn status_id(self) -> i64 {
        match self {
            ReadingStatus::WantToRead => 1,
            ReadingStatus::CurrentlyReading => 2,
            ReadingStatus::Read => 3,
            // NOTE: status_id 4 is not used for a user-facing shelf; DNF is 5.
            ReadingStatus::Dnf => 5,
        }
    }

    /// Parse a Hardcover `status_id` back into a [`ReadingStatus`].
    pub fn from_status_id(id: i64) -> Option<Self> {
        match id {
            1 => Some(ReadingStatus::WantToRead),
            2 => Some(ReadingStatus::CurrentlyReading),
            3 => Some(ReadingStatus::Read),
            5 => Some(ReadingStatus::Dnf),
            _ => None,
        }
    }
}

/// Settings for the Hardcover connector.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HardcoverConfig {
    /// User-supplied API key (from the Hardcover account settings page).
    ///
    /// Sent as `Authorization: Bearer {api_key}`. If the stored value already
    /// begins with `Bearer ` it is used as-is (Hardcover sometimes displays the
    /// token pre-prefixed).
    pub api_key: String,
}

/// The Hardcover connector.
pub struct HardcoverProvider {
    config: HardcoverConfig,
    /// The authenticated user's Hardcover id, resolved during [`Self::authenticate`].
    user_id: Option<i64>,
    client: reqwest::Client,
}

impl HardcoverProvider {
    pub const ID: &'static str = "hardcover";

    pub fn new(config: HardcoverConfig) -> Self {
        Self {
            config,
            user_id: None,
            client: reqwest::Client::new(),
        }
    }

    /// The value for the `Authorization` header, tolerating a pre-prefixed token.
    fn auth_header(&self) -> String {
        let key = self.config.api_key.trim();
        if key.len() >= 7 && key[..7].eq_ignore_ascii_case("bearer ") {
            key.to_string()
        } else {
            format!("Bearer {key}")
        }
    }

    /// POST a GraphQL operation and deserialize its `data` payload as `T`.
    ///
    /// Handles transport errors, the documented HTTP status codes, and the
    /// standard `{ data, errors }` envelope (a non-empty `errors` array becomes a
    /// [`ProviderError::Api`]).
    async fn post<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> ProviderResult<T> {
        let body = serde_json::json!({ "query": query, "variables": variables });

        let resp = self
            .client
            .post(HARDCOVER_GRAPHQL_ENDPOINT)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        match status {
            200 => parse_graphql::<T>(&text),
            401 => Err(ProviderError::NotAuthenticated),
            403 => Err(ProviderError::Api(extract_api_error(&text).unwrap_or_else(
                || "forbidden (403): no access to the requested resource".into(),
            ))),
            429 => Err(ProviderError::Api(
                "Hardcover rate limit exceeded (429 Throttled); retry later".into(),
            )),
            other => Err(ProviderError::Other(format!(
                "unexpected status {other} from Hardcover: {text}"
            ))),
        }
    }

    /// Search Hardcover for books matching `query` and return the resolved ids.
    ///
    /// Uses the official `search` query (Typesense-backed). Returns the ordered
    /// `ids` list; callers typically take the first as the best match.
    pub async fn search_book_ids(&self, query: &str, per_page: u32) -> ProviderResult<Vec<i64>> {
        let data: SearchData = self
            .post(
                SEARCH_QUERY,
                serde_json::json!({
                    "query": query,
                    "queryType": "book",
                    "perPage": per_page,
                    "page": 1,
                }),
            )
            .await?;
        Ok(search_ids(&data.search))
    }

    /// Resolve a normalized [`Book`] to a Hardcover book id.
    ///
    /// Prefers an ISBN/ASIN identifier when present (most precise), otherwise
    /// falls back to a `title author` text search. Returns `None` when nothing
    /// matches — the caller decides whether that's an error.
    pub async fn resolve_book_id(&self, book: &Book) -> ProviderResult<Option<i64>> {
        let query = book
            .identifiers
            .get("isbn")
            .or_else(|| book.identifiers.get("asin"))
            .cloned()
            .unwrap_or_else(|| {
                let author = book.authors.first().map(String::as_str).unwrap_or("");
                format!("{} {}", book.title, author).trim().to_string()
            });
        Ok(self.search_book_ids(&query, 5).await?.into_iter().next())
    }

    /// Set (upsert) the reading status for a Hardcover book, optionally with a
    /// rating, via the `insert_user_book` mutation.
    pub async fn set_reading_status(
        &self,
        book_id: i64,
        status: ReadingStatus,
        rating: Option<f32>,
    ) -> ProviderResult<UserBookMutationResult> {
        let mut object = serde_json::json!({
            "book_id": book_id,
            "status_id": status.status_id(),
        });
        if let Some(r) = rating {
            object["rating"] = serde_json::json!(r);
        }
        let data: serde_json::Value = self
            .post(INSERT_USER_BOOK_MUTATION, serde_json::json!({ "object": object }))
            .await?;
        parse_user_book_mutation(&data, "insert_user_book")
    }

    /// Set the rating (0.0–5.0) for an existing shelf entry via `update_user_book`.
    pub async fn set_rating(
        &self,
        user_book_id: i64,
        rating: f32,
    ) -> ProviderResult<UserBookMutationResult> {
        let data: serde_json::Value = self
            .post(
                UPDATE_USER_BOOK_MUTATION,
                serde_json::json!({
                    "id": user_book_id,
                    "object": { "rating": rating },
                }),
            )
            .await?;
        parse_user_book_mutation(&data, "update_user_book")
    }

    /// Record reading progress (pages and/or seconds) for a shelf entry.
    ///
    /// TODO(hardcover): the current docs describe progress as `user_book_read`
    /// records but the exact input type for `insert_user_book_read` is in flux.
    /// This implements the closest documented form (a `user_book_read` object
    /// carrying `progress_pages`/`progress_seconds`); verify against the live
    /// schema once an API key is available.
    pub async fn update_progress(
        &self,
        user_book_id: i64,
        progress_pages: Option<i64>,
        progress_seconds: Option<i64>,
    ) -> ProviderResult<UserBookMutationResult> {
        let mut read = serde_json::Map::new();
        if let Some(p) = progress_pages {
            read.insert("progress_pages".into(), serde_json::json!(p));
        }
        if let Some(s) = progress_seconds {
            read.insert("progress_seconds".into(), serde_json::json!(s));
        }
        let data: serde_json::Value = self
            .post(
                INSERT_USER_BOOK_READ_MUTATION,
                serde_json::json!({
                    "userBookId": user_book_id,
                    "userBookRead": serde_json::Value::Object(read),
                }),
            )
            .await?;
        parse_user_book_mutation(&data, "insert_user_book_read")
    }
}

#[async_trait]
impl Provider for HardcoverProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    fn display_name(&self) -> &str {
        "Hardcover"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // Reading-tracker only: read/write reading status, ratings, progress.
        ProviderCapabilities::PROGRESS_SYNC
    }

    async fn authenticate(&mut self, config: &serde_json::Value) -> ProviderResult<()> {
        if !config.is_null() {
            self.config = serde_json::from_value(config.clone())
                .map_err(|e| ProviderError::Config(e.to_string()))?;
        }
        if self.config.api_key.trim().is_empty() {
            return Err(ProviderError::Config("api_key is empty".into()));
        }

        // Confirm the token by resolving the current user.
        let data: MeData = self.post(ME_QUERY, serde_json::json!({})).await?;
        let user = first_user(data)?;
        self.user_id = Some(user.id);
        Ok(())
    }

    async fn list_library(&self) -> ProviderResult<Vec<Book>> {
        let user_id = self.user_id.ok_or(ProviderError::NotAuthenticated)?;

        // Read the user's shelves (their own tracked books) for read-back.
        let data: UserBooksData = self
            .post(USER_BOOKS_QUERY, serde_json::json!({ "userId": user_id }))
            .await?;

        Ok(data
            .user_books
            .iter()
            .filter_map(map_user_book_to_book)
            .collect())
    }
}

// ---------------------------------------------------------------------------
// GraphQL operations (documented shapes; queries use variables throughout).
// ---------------------------------------------------------------------------

const ME_QUERY: &str = r#"query Me { me { id username } }"#;

const USER_BOOKS_QUERY: &str = r#"
query UserBooks($userId: Int!) {
  user_books(where: { user_id: { _eq: $userId } }) {
    id
    status_id
    rating
    book {
      id
      title
      contributions { author { name } }
      image { url }
    }
  }
}
"#;

const SEARCH_QUERY: &str = r#"
query Search($query: String!, $queryType: String, $perPage: Int, $page: Int) {
  search(query: $query, query_type: $queryType, per_page: $perPage, page: $page) {
    ids
    results
  }
}
"#;

const INSERT_USER_BOOK_MUTATION: &str = r#"
mutation InsertUserBook($object: UserBookCreateInput!) {
  insert_user_book(object: $object) {
    id
    error
    user_book { id book_id status_id rating }
  }
}
"#;

const UPDATE_USER_BOOK_MUTATION: &str = r#"
mutation UpdateUserBook($id: Int!, $object: UserBookUpdateInput!) {
  update_user_book(id: $id, object: $object) {
    id
    error
    user_book { id book_id status_id rating }
  }
}
"#;

// TODO(hardcover): confirm the exact input type name for reading-progress
// records against the live schema (in flux). `insert_user_book_read` is the
// closest documented form.
const INSERT_USER_BOOK_READ_MUTATION: &str = r#"
mutation InsertUserBookRead($userBookId: Int!, $userBookRead: DatesReadInput!) {
  insert_user_book_read(user_book_id: $userBookId, user_book_read: $userBookRead) {
    id
    error
    user_book { id book_id status_id }
  }
}
"#;

// ---------------------------------------------------------------------------
// Response envelope + payload types.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GraphQLResponse<T> {
    #[serde(default = "Option::default")]
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct MeData {
    #[serde(default)]
    me: Vec<HardcoverUser>,
}

/// A minimal view of the authenticated Hardcover user.
#[derive(Debug, Clone, Deserialize)]
pub struct HardcoverUser {
    pub id: i64,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserBooksData {
    #[serde(default)]
    user_books: Vec<UserBookRow>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct UserBookRow {
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<i64>,
    #[serde(default)]
    status_id: Option<i64>,
    #[serde(default)]
    #[allow(dead_code)]
    rating: Option<f64>,
    #[serde(default)]
    book: Option<HardcoverBook>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct HardcoverBook {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    contributions: Vec<Contribution>,
    #[serde(default)]
    image: Option<HcImage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Contribution {
    #[serde(default)]
    author: Option<Author>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Author {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct HcImage {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchData {
    #[serde(default)]
    search: Option<SearchPayload>,
}

#[derive(Debug, Default, Deserialize)]
struct SearchPayload {
    /// Ordered result ids (may arrive as numbers or numeric strings).
    #[serde(default)]
    ids: Vec<serde_json::Value>,
    /// Raw Typesense result object (kept opaque; we only need `ids` for resolve).
    #[serde(default)]
    #[allow(dead_code)]
    results: serde_json::Value,
}

/// The result of a `*_user_book*` mutation.
///
/// Hardcover's write mutations return a small envelope with an optional custom
/// `error` string (separate from the top-level GraphQL `errors` array).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserBookMutationResult {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub user_book: Option<UserBookRef>,
}

/// The `user_book` row echoed back by a mutation.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserBookRef {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub book_id: Option<i64>,
    #[serde(default)]
    pub status_id: Option<i64>,
    #[serde(default)]
    pub rating: Option<f64>,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without any network access).
// ---------------------------------------------------------------------------

/// Parse the `{ data, errors }` envelope, turning a non-empty `errors` array
/// into a [`ProviderError::Api`] and a missing `data` into [`ProviderError::Other`].
fn parse_graphql<T: DeserializeOwned>(body: &str) -> ProviderResult<T> {
    let env: GraphQLResponse<T> = serde_json::from_str(body)
        .map_err(|e| ProviderError::Other(format!("invalid GraphQL response: {e}")))?;

    if let Some(errors) = env.errors {
        if !errors.is_empty() {
            let msg = errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; ");
            return Err(ProviderError::Api(msg));
        }
    }

    env.data
        .ok_or_else(|| ProviderError::Other("GraphQL response contained no data".into()))
}

/// Best-effort extraction of the `{ "error": "..." }` body Hardcover returns for
/// some non-200 responses.
fn extract_api_error(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_string)
}

/// Take the first user from a `me` payload (the query returns an array).
fn first_user(data: MeData) -> ProviderResult<HardcoverUser> {
    data.me
        .into_iter()
        .next()
        .ok_or_else(|| ProviderError::Api("Hardcover `me` returned no user".into()))
}

/// Extract ordered book ids from a search payload, coercing numeric strings.
fn search_ids(search: &Option<SearchPayload>) -> Vec<i64> {
    let Some(payload) = search else {
        return Vec::new();
    };
    payload
        .ids
        .iter()
        .filter_map(|v| match v {
            serde_json::Value::Number(n) => n.as_i64(),
            serde_json::Value::String(s) => s.parse::<i64>().ok(),
            _ => None,
        })
        .collect()
}

/// Map a Hardcover shelf row into a normalized [`Book`] for read-back.
///
/// Returns `None` when the row has no usable book/title. `status_id` is folded
/// into [`Progress`]: `Read` → finished, `Currently Reading` → in-progress.
fn map_user_book_to_book(row: &UserBookRow) -> Option<Book> {
    let book = row.book.as_ref()?;
    let id = book.id?;
    let title = book.title.clone().filter(|t| !t.is_empty())?;

    let authors: Vec<String> = book
        .contributions
        .iter()
        .filter_map(|c| c.author.as_ref().and_then(|a| a.name.clone()))
        .filter(|n| !n.is_empty())
        .collect();

    let progress = row.status_id.and_then(ReadingStatus::from_status_id).map(|s| {
        let finished = matches!(s, ReadingStatus::Read);
        Progress {
            fraction: if finished { 1.0 } else { 0.0 },
            position_seconds: None,
            finished,
        }
    });

    let mut out = Book::new(
        id.to_string(),
        title,
        MediaType::Ebook,
        HardcoverProvider::ID,
    );
    out.authors = authors;
    out.cover_url = book
        .image
        .as_ref()
        .and_then(|i| i.url.clone())
        .filter(|u| !u.is_empty());
    out.progress = progress;
    Some(out)
}

/// Parse a `*_user_book*` mutation result out of a GraphQL `data` object,
/// surfacing the mutation's own `error` field as a [`ProviderError::Api`].
fn parse_user_book_mutation(
    data: &serde_json::Value,
    field: &str,
) -> ProviderResult<UserBookMutationResult> {
    let node = data
        .get(field)
        .ok_or_else(|| ProviderError::Other(format!("mutation response missing `{field}`")))?;
    let result: UserBookMutationResult = serde_json::from_value(node.clone())
        .map_err(|e| ProviderError::Other(format!("invalid `{field}` response: {e}")))?;
    if let Some(err) = &result.error {
        if !err.is_empty() {
            return Err(ProviderError::Api(err.clone()));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_me_and_takes_first_user() {
        let body = r#"{ "data": { "me": [ { "id": 12345, "username": "alice" } ] } }"#;
        let data: MeData = parse_graphql(body).unwrap();
        let user = first_user(data).unwrap();
        assert_eq!(user.id, 12345);
        assert_eq!(user.username.as_deref(), Some("alice"));
    }

    #[test]
    fn graphql_errors_become_typed_api_error() {
        let body = r#"{ "errors": [ { "message": "field 'nope' not found" },
                                     { "message": "second problem" } ] }"#;
        let err = parse_graphql::<MeData>(body).unwrap_err();
        match err {
            ProviderError::Api(msg) => {
                assert!(msg.contains("field 'nope' not found"));
                assert!(msg.contains("second problem"));
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn resolves_first_book_id_from_search_results() {
        // Representative `search` payload: `ids` ordered by match, `results` is
        // the opaque Typesense object (only `ids` is needed to resolve).
        let body = r#"{
          "data": {
            "search": {
              "ids": [56789, 111, 222],
              "results": {
                "found": 3,
                "hits": [
                  { "document": { "id": "56789", "title": "The Hobbit",
                                  "author_names": ["J. R. R. Tolkien"] } }
                ]
              }
            }
          }
        }"#;
        let data: SearchData = parse_graphql(body).unwrap();
        let ids = search_ids(&data.search);
        assert_eq!(ids, vec![56789, 111, 222]);
        assert_eq!(ids.first().copied(), Some(56789));
    }

    #[test]
    fn maps_user_book_row_to_book_with_authors_cover_and_progress() {
        let body = r#"{
          "data": {
            "user_books": [
              {
                "id": 901,
                "status_id": 3,
                "rating": 4.5,
                "book": {
                  "id": 56789,
                  "title": "The Hobbit",
                  "contributions": [
                    { "author": { "name": "J. R. R. Tolkien" } }
                  ],
                  "image": { "url": "https://assets.hardcover.app/covers/hobbit.jpg" }
                }
              },
              { "id": 902, "status_id": 2, "rating": null, "book": null }
            ]
          }
        }"#;
        let data: UserBooksData = parse_graphql(body).unwrap();
        let books: Vec<Book> = data.user_books.iter().filter_map(map_user_book_to_book).collect();

        // The second row has no `book` and is dropped.
        assert_eq!(books.len(), 1);
        let b = &books[0];
        assert_eq!(b.id, "56789");
        assert_eq!(b.title, "The Hobbit");
        assert_eq!(b.authors, vec!["J. R. R. Tolkien"]);
        assert_eq!(b.source_provider_id, "hardcover");
        assert_eq!(b.media_type, MediaType::Ebook);
        assert_eq!(
            b.cover_url.as_deref(),
            Some("https://assets.hardcover.app/covers/hobbit.jpg")
        );
        let p = b.progress.as_ref().expect("status_id 3 -> progress");
        assert!(p.finished);
        assert!((p.fraction - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_successful_mutation_result() {
        let body = r#"{
          "data": {
            "insert_user_book": {
              "id": 901,
              "error": null,
              "user_book": { "id": 901, "book_id": 56789, "status_id": 2, "rating": null }
            }
          }
        }"#;
        // Emulate `post` returning the `data` object.
        let data: serde_json::Value = {
            let env: GraphQLResponse<serde_json::Value> = serde_json::from_str(body).unwrap();
            env.data.unwrap()
        };
        let result = parse_user_book_mutation(&data, "insert_user_book").unwrap();
        assert_eq!(result.id, Some(901));
        let ub = result.user_book.expect("user_book echoed");
        assert_eq!(ub.book_id, Some(56789));
        assert_eq!(ub.status_id, Some(2));
    }

    #[test]
    fn mutation_custom_error_field_becomes_api_error() {
        let data = serde_json::json!({
            "insert_user_book": { "id": null, "error": "book_id is required", "user_book": null }
        });
        let err = parse_user_book_mutation(&data, "insert_user_book").unwrap_err();
        match err {
            ProviderError::Api(msg) => assert_eq!(msg, "book_id is required"),
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn reading_status_round_trips_through_status_id() {
        for s in [
            ReadingStatus::WantToRead,
            ReadingStatus::CurrentlyReading,
            ReadingStatus::Read,
            ReadingStatus::Dnf,
        ] {
            assert_eq!(ReadingStatus::from_status_id(s.status_id()), Some(s));
        }
        assert_eq!(ReadingStatus::from_status_id(4), None);
    }

    #[test]
    fn auth_header_prefixes_bearer_and_tolerates_existing_prefix() {
        let bare = HardcoverProvider::new(HardcoverConfig { api_key: "abc123".into() });
        assert_eq!(bare.auth_header(), "Bearer abc123");
        let prefixed = HardcoverProvider::new(HardcoverConfig {
            api_key: "Bearer abc123".into(),
        });
        assert_eq!(prefixed.auth_header(), "Bearer abc123");
    }
}
