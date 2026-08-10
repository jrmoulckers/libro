# ADR-0001: Pure-client architecture with no server tier

## Status

Accepted

## Context

libro is a cross-platform media hub for books, audiobooks, and a personal library. Its
`AGENTS.md` has always asserted "No server tier, ever" as the repository's hardest constraint,
and most other product rules depend on it: the secrets rule, the on-device data rule, the
offline rule, and the entire `ENG-INT-*` deviation all follow from having no server.

That assertion was never recorded as an architectural decision.
[`ENG-ARCH-003`](https://github.com/jrmoulckers/engineering/blob/main/principles/architecture/boundaries-and-contracts.md) _Durable decisions_ requires that consequential architectural tradeoffs be recorded as ADRs
**before** they are treated as durable constraints. libro was treating this one as durable —
declining features on the strength of it — while citing the principle that demands the record.
Citing an obligation a repository does not satisfy is worse than not citing it, because the
citation makes the gap look closed.

This ADR supplies the missing record. It introduces no new constraint and changes no behavior;
it makes an existing constraint auditable and states the tradeoff that was accepted.

Two alternatives were considered: a thin backend-for-frontend to proxy third-party metadata
providers, and static hosting plus serverless functions for the same purpose. Both were
rejected. Each creates an operational surface, a deployment dependency, and a place where user
library data could accumulate, in exchange for conveniences libro can otherwise obtain by asking
the user for their own credentials.

## Decision

libro ships as a static bundle with no server tier. Concretely:

- No API route, Node server, SSR, edge function, or backend-for-frontend is added to this
  repository, and none is depended upon at runtime.
- The build output is a static `dist/` deployable to any static host or CDN. Nothing in the
  request path is owned by this project.
- The device's durable store is the system of record
  ([`ENG-LOCAL-001`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md)).
- Third-party integrations must work against public endpoints, or against credentials the user
  supplies that remain in device storage. A feature requiring a private API key is out of scope
  by construction, because there is nowhere to inject one at runtime.
- If a proposed feature appears to require a server, the feature is redesigned or declined. The
  constraint is not traded away per feature.

Reversing this decision requires a superseding ADR, not a pull request that happens to add a
route.

## Consequences

**Accepted costs.** Capabilities that genuinely require a trusted server are permanently out of
scope: cross-device sync brokered by libro itself, server-side metadata enrichment behind a
private key, and any shared or multi-user surface.
[`ENG-INT-005`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/integration-boundaries.md)
(third-party credentials behind a server-side proxy) is unsatisfiable here by construction, and
is recorded as a deviation in `AGENTS.md` rather than tracked as an outstanding defect.

**What it buys.** There is no server to breach, no deployment to operate, and no destination to
which user library data could be exfiltrated — the strongest available reading of
[`ENG-SEC-001`](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/security-and-privacy.md)
and
[`ENG-SEC-008`](https://github.com/jrmoulckers/engineering/blob/main/principles/assurance/security-and-privacy.md).
Offline operation is the default rather than a degraded mode
([`ENG-LOCAL-004`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md)).

**What it obligates.** Because everything runs in the browser, the client bundle is the entire
performance surface, so the delivery budget in
[`ENG-WEB-003`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/browser-frontend.md)
is enforced in CI rather than left advisory. Because the bundle is also the entire threat
surface,
[`ENG-WEB-001`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/browser-frontend.md)
governs all untrusted input reaching the browser.

**What it does not exempt.** libro has no _server_ database, but
[`ENG-DATA-001`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/data-systems.md)
is scoped by durability rather than by tier, so the planned IndexedDB library index is fully
governed by it. Sync, if ever added, must be an optional seam over a provider the user chooses
([`ENG-LOCAL-002`](https://github.com/jrmoulckers/engineering/blob/main/principles/platforms/local-first.md)),
never a service this repository operates.
