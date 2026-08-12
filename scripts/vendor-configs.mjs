#!/usr/bin/env node
/**
 * Vendor the dependency-free shared configuration from `jrmoulckers/engineering`
 * at a pinned ref, without a package registry.
 *
 * Why this exists: GitHub Packages authenticates *every* read, including reads
 * of a public package. For a self-hosted product that means each contributor
 * and each self-hoster must mint a token before `install` succeeds — a real
 * onboarding regression, and one the package-visibility setting does not fix.
 * `@jrmoulckers/tsconfig` and `@jrmoulckers/prettier-config` have no runtime
 * dependencies, so they can be fetched directly and committed.
 *
 * `@jrmoulckers/eslint-config` is deliberately NOT vendorable here: it depends
 * on `@eslint/js`, `typescript-eslint`, `eslint-config-prettier` and `globals`
 * at runtime. Copying its source would push four version choices back onto
 * every consumer, which is the drift the shared layer exists to remove. Install
 * that one from the registry.
 *
 * Vendoring usually trades away the version signal a registry gives you. It
 * does not here: every fetch writes `engineering-configs.lock.json` recording
 * the ref and the SHA-256 of each file, so drift is detectable and a refresh is
 * a reviewable diff.
 *
 * Usage:
 *   node scripts/vendor-configs.mjs <ref> [--dest <dir>] [--set tsconfig,prettier]
 *
 * Files are written byte-identical to source — no generated header — so that
 * `git diff` after a re-run shows exactly what upstream changed and nothing
 * else. Provenance lives in the lock file instead.
 */

import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join, dirname, resolve } from 'node:path';

const REPO = 'jrmoulckers/engineering';
const LOCK = 'engineering-configs.lock.json';

const SETS = {
  tsconfig: {
    // `extends` between these is relative, so a fetch must take the whole
    // transitive closure of what it uses or the config resolves to nothing.
    // The closure libro actually reaches is base + vite-app + vite-node;
    // next.json, vite-react.json and node.json are carried but unreachable.
    // They are kept because the lock hashes all six, so an unused file cannot
    // drift silently — it is reviewed on refresh rather than trusted. Trim the
    // list, not the closure, if that review cost ever outweighs the option.
    from: 'packages/tsconfig',
    files: [
      'base.json',
      'vite-app.json',
      'vite-node.json',
      'vite-react.json',
      'next.json',
      'node.json',
    ],
  },
  prettier: {
    from: 'packages/prettier-config',
    files: ['index.js', 'svelte.js'],
  },
};

class VendorError extends Error {
  constructor(message, hint) {
    super(message);
    this.hint = hint;
  }
}

/**
 * Throw rather than `process.exit()`. Exiting from inside an in-flight `fetch`
 * tears down a socket the runtime still owns, which on Windows surfaces as a
 * libuv assertion and a 0xC0000409 exit code instead of the message and the 1
 * that a consumer's CI can act on.
 */
function fail(message, hint) {
  throw new VendorError(message, hint);
}

function parseArgs(argv) {
  const positional = [];
  const flags = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--dest' || arg === '--set') {
      const value = argv[i + 1];
      if (!value || value.startsWith('--')) fail(`${arg} requires a value`);
      flags[arg.slice(2)] = value;
      i += 1;
    } else if (arg === '--check') {
      flags.check = true;
    } else if (arg === '--no-remote') {
      flags.noRemote = true;
    } else if (arg.startsWith('--')) {
      fail(
        `unknown option ${arg}`,
        'Usage: vendor-configs.mjs <ref> [--dest <dir>] [--set a,b] | vendor-configs.mjs --check [--no-remote]',
      );
    } else {
      positional.push(arg);
    }
  }
  return { positional, flags };
}

/**
 * Report whether a newer release exists. Never throws and never fails the
 * caller: a tag pushed upstream must not turn an unrelated PR red.
 *
 * Returns a discriminated result rather than a bare tag, because "no newer
 * release" and "could not ask" are different answers that used to share one
 * output: silence. An offline, rate-limited, or unauthenticated run printed
 * exactly what an up-to-date pin printed, so the reassuring case was
 * indistinguishable from the case where nothing was compared at all. This is
 * not hypothetical here -- the first attempt to verify the ordering fix was a
 * dead control for precisely this reason (403 API rate limit exceeded).
 */
async function latestRef() {
  try {
    const response = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { accept: 'application/vnd.github+json' },
    });
    if (!response.ok) {
      return { ok: false, reason: `HTTP ${response.status} from the releases API` };
    }
    const body = await response.json();
    if (typeof body.tag_name !== 'string') {
      return { ok: false, reason: 'the releases API returned no tag_name' };
    }
    return { ok: true, ref: body.tag_name };
  } catch (error) {
    return { ok: false, reason: error?.message ?? 'the releases API was unreachable' };
  }
}

/**
 * Order two version-shaped tags. `releases/latest` is ordered by tag date, not
 * by version, so a backported patch on an older line can be "latest" while
 * being older than the pin. An inequality test then advertises it as an
 * update, which for a lock file is a downgrade — the one direction that fails
 * silently, because the older payload was correct when it was current.
 * Returns >0 when `a` is newer, and 0 when the two cannot be compared.
 */
function compareRefs(a, b) {
  const parse = (ref) => {
    const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(ref.trim());
    return match ? match.slice(1, 4).map(Number) : null;
  };
  const left = parse(a);
  const right = parse(b);
  if (!left || !right) return a === b ? 0 : 1;
  for (let i = 0; i < 3; i += 1) {
    if (left[i] !== right[i]) return left[i] - right[i];
  }
  return 0;
}

/**
 * Verify the vendored tree still matches the lock, then report staleness.
 *
 * The split in severity is the whole point. Drift is a local integrity failure
 * — someone edited a generated file, or a write was lost — so it exits non-zero.
 * Staleness is an upstream event the consumer has not acted on yet, so it only
 * warns. Failing on staleness would make pinning automatic in effect: a red
 * build pressures the next person into bumping the ref without deciding to
 * accept the change, which is the property pinning exists to protect.
 *
 * These are also two unrelated mechanisms behind one flag: the hash comparison
 * is offline and authoritative, the staleness notice is an unauthenticated call
 * to api.github.com on every lint. `--no-remote` drops the second and keeps the
 * first. A green run means the tree matches the lock; it never means the pin is
 * current.
 */
async function check(noRemote = false) {
  let lock;
  try {
    lock = JSON.parse(await readFile(LOCK, 'utf8'));
  } catch {
    fail(`no ${LOCK} found`, 'Run: node scripts/vendor-configs.mjs <ref>');
  }

  const entries = Object.entries(lock.files ?? {});
  if (entries.length === 0) fail(`${LOCK} records no files`, 'Re-run the vendor step.');

  // An absolute key means the lock was written by a run with --dest pointing
  // outside the repo. Every path still resolves on the machine that wrote it,
  // so --check passes while examining no file in the repository at all — the
  // guard is disarmed rather than weakened, and it fails only on a runner,
  // as `missing` on a path that never existed there.
  const absolute = entries.map(([dest]) => dest).filter((dest) => /^([A-Za-z]:[/\\]|[/\\])/.test(dest));
  if (absolute.length > 0) {
    fail(
      `${LOCK} records ${absolute.length} absolute path(s), so it guards nothing here:\n  ${absolute.join('\n  ')}`,
      `Written by a --dest run outside the repo. Re-vendor without --dest: node scripts/vendor-configs.mjs ${lock.ref}`,
    );
  }

  const drifted = [];
  for (const [dest, meta] of entries) {
    let text;
    try {
      text = await readFile(dest, 'utf8');
    } catch {
      drifted.push(`${dest}: missing`);
      continue;
    }
    if (sha256(text) !== meta.sha256) drifted.push(`${dest}: content differs from the lock`);
  }

  if (drifted.length > 0) {
    fail(
      `${drifted.length} vendored file(s) drifted from ${LOCK}:\n  ${drifted.join('\n  ')}`,
      `These files are generated. Do not edit them — re-run: node scripts/vendor-configs.mjs ${lock.ref}`,
    );
  }

  // The loop above compares the lock against the disk, so it can only see files
  // the lock already names. SETS is what decides that set, and it is edited by
  // hand — so a file added to SETS and never vendored is absent from the lock,
  // absent from disk, and invisible to every comparison above. --check then
  // passes while the tree is missing a file the manifest says belongs there.
  // Compare the two directly: the lock is evidence about a file set, and only
  // SETS says which set that should be.
  const expected = Object.entries(SETS).flatMap(([name, set]) =>
    set.files.map((file) => join(DEFAULT_DEST, name, file).split('\\').join('/')),
  );
  const locked = new Set(entries.map(([dest]) => dest.split('\\').join('/')));
  const unvendored = expected.filter((dest) => !locked.has(dest));
  const orphaned = [...locked].filter((dest) => !expected.includes(dest));

  if (unvendored.length > 0 || orphaned.length > 0) {
    const lines = [
      ...unvendored.map((d) => `${d}: named by SETS, absent from the lock`),
      ...orphaned.map((d) => `${d}: in the lock, no longer named by SETS`),
    ];
    fail(
      `${LOCK} does not match the manifest in scripts/vendor-configs.mjs:\n  ${lines.join('\n  ')}`,
      `SETS changed without a re-vendor, so the lock describes the previous file set. ` +
        `Re-run: node scripts/vendor-configs.mjs ${lock.ref}`,
    );
  }

  process.stdout.write(`${entries.length} vendored file(s) match ${LOCK} at ${lock.ref}.\n`);

  if (noRemote) {
    process.stdout.write(
      `\nStaleness not checked (--no-remote). This says nothing about whether ${lock.ref} is current.\n`,
    );
    return;
  }

  const latest = await latestRef();
  if (!latest.ok) {
    process.stdout.write(
      `\nStaleness could not be checked: ${latest.reason}.\n` +
        `The hash comparison above is unaffected -- it is offline and authoritative.\n` +
        `This says nothing about whether ${lock.ref} is current.\n`,
    );
    return;
  }
  if (compareRefs(latest.ref, lock.ref) > 0) {
    process.stdout.write(
      `\nNotice: pinned at ${lock.ref}; newest release is ${latest.ref}.\n` +
        `This is not a failure. Update deliberately when you choose to:\n` +
        `  node scripts/vendor-configs.mjs ${latest.ref}\n`,
    );
  }
}

/**
 * A fetch can fail in three ways and only the first is obvious. A non-200 is
 * loud. An empty 200 is quiet. A 200 carrying the wrong bytes — an HTML error
 * page, a redirect landing page, an LFS pointer — is silent, and it is the one
 * that leaves a file on disk that tools then "successfully" read as empty
 * configuration. All three are fatal here.
 */
function assertPayload(path, text) {
  if (text.trim() === '') {
    fail(`${path} came back empty`, 'The ref may exist but not contain this file.');
  }
  if (path.endsWith('.json')) {
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch {
      fail(
        `${path} is not valid JSON`,
        'This is usually an HTML error page served with status 200.',
      );
    }
    if (!parsed || typeof parsed !== 'object' || !parsed.compilerOptions) {
      fail(
        `${path} has no "compilerOptions"`,
        'It parsed, but it is not a TypeScript configuration.',
      );
    }
  } else if (!/^export /m.test(text)) {
    fail(`${path} exports nothing`, 'It downloaded, but it is not an ES module configuration.');
  }
}

async function fetchFile(ref, path) {
  const url = `https://raw.githubusercontent.com/${REPO}/${ref}/${path}`;
  let response;
  try {
    response = await fetch(url);
  } catch (cause) {
    fail(`could not reach ${url}`, String(cause.message ?? cause));
  }
  if (!response.ok) {
    fail(
      `${url} returned HTTP ${response.status}`,
      `Check that ref '${ref}' exists in ${REPO} and contains this path.`,
    );
  }
  const text = await response.text();
  assertPayload(path, text);
  return text;
}

const sha256 = (text) => createHash('sha256').update(text, 'utf8').digest('hex');

const DEFAULT_DEST = 'config/engineering';

async function main() {
  const { positional, flags } = parseArgs(process.argv.slice(2));
  if (flags.check) {
    if (positional.length > 0) {
      fail('--check takes no ref', 'It verifies the ref already recorded in the lock file.');
    }
    await check(flags.noRemote === true);
    return;
  }
  const ref = positional[0];
  if (!ref) {
    fail('a ref is required', 'Pass a tag, not a branch: node scripts/vendor-configs.mjs v1.2.3');
  }
  const dest = flags.dest ?? DEFAULT_DEST;
  const names = (flags.set ?? Object.keys(SETS).join(',')).split(',').map((s) => s.trim());
  for (const name of names) {
    if (!SETS[name]) fail(`unknown set '${name}'`, `Known sets: ${Object.keys(SETS).join(', ')}`);
  }

  // Fetch and validate everything before writing anything. A partial write is
  // worse than a failed one: the tools would run against a mix of refs and
  // report success.
  const staged = [];
  for (const name of names) {
    const { from, files } = SETS[name];
    for (const file of files) {
      const path = `${from}/${file}`;
      const text = await fetchFile(ref, path);
      staged.push({ name, path, file, text, dest: join(dest, name, file) });
    }
  }

  for (const item of staged) {
    await mkdir(dirname(item.dest), { recursive: true });
    await writeFile(item.dest, item.text, 'utf8');
  }

  const lock = {
    repository: REPO,
    ref,
    fetchedAt: new Date().toISOString(),
    refresh: `node scripts/vendor-configs.mjs <newer-ref>`,
    files: Object.fromEntries(
      staged.map((item) => [
        item.dest.split('\\').join('/'),
        { source: item.path, sha256: sha256(item.text) },
      ]),
    ),
  };

  let previous = null;
  try {
    previous = JSON.parse(await readFile(LOCK, 'utf8'));
  } catch {
    // No previous lock: this is a first vendor.
  }

  // Key the comparison by upstream source path, never by destination. A
  // dest-keyed lookup misses on every entry as soon as --dest moves, turning
  // the count into "all changed" — meaningless in the one place the evaluation
  // recipe tells you to read it.
  const priorBySource = new Map(
    Object.values(previous?.files ?? {}).map((meta) => [meta.source, meta.sha256]),
  );
  const changed = staged.filter((item) => priorBySource.get(item.path) !== sha256(item.text));

  // Key the evaluation mode on where the files actually landed, not on whether
  // the flag was passed. Upstream's own refresh recipe passes the real path
  // explicitly (`--dest config/engineering`), and keying on flag presence
  // treated that as scratch: the tree was rewritten in place while the lock
  // kept the old ref. Benign only while the payload is identical.
  const scratch = resolve(dest) !== resolve(DEFAULT_DEST);
  if (!scratch) {
    await writeFile(LOCK, `${JSON.stringify(lock, null, 2)}\n`, 'utf8');
  }

  process.stdout.write(`Vendored ${staged.length} file(s) from ${REPO}@${ref} into ${dest}/\n`);
  if (previous && previous.ref !== ref) {
    process.stdout.write(
      `Ref moved ${previous.ref} -> ${ref}; ${changed.length} file(s) changed content.\n`,
    );
  }
  if (scratch) {
    process.stdout.write(
      `Evaluation run: --dest pointed outside ${DEFAULT_DEST}, so ${LOCK} was left untouched and nothing here is committable.\n` +
        `Re-run without --dest, or with --dest ${DEFAULT_DEST}, to adopt this ref.\n`,
    );
    return;
  }
  process.stdout.write(`Recorded ref and SHA-256 of each file in ${LOCK}. Commit both.\n`);
}

try {
  await main();
} catch (error) {
  if (!(error instanceof VendorError)) throw error;
  process.stderr.write(`error: ${error.message}\n`);
  if (error.hint) process.stderr.write(`       ${error.hint}\n`);
  process.exitCode = 1;
}
