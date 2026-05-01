# shmemo

*shmemo* is a small program that memoizes (caches) shell command executions.

Run a command through `shmemo` once and it records:

- `stdout`
- `stderr`
- exit code

Then, when you run the same command again, `shmemo` instantly replays the cached
output instead of re-running the command.

## Why

Useful for commands that are expensive, flaky, rate-limited, or slow (e.g.
network calls), when you want fast repeatable runs during development.

## Install

### From source (recommended)

```bash
cargo install --git https://github.com/unprofessor/shmemo.git --locked
```

### Build a release binary

```bash
cargo build --release
./target/release/shmemo --help
```

### Using GNU Guix

To build with a reproducible environment using [GNU Guix](https://guix.gnu.org/):

```bash
# Enter a development shell
guix time-machine -C channels.scm -- shell -m manifest.scm
```

## Usage

### Basic

```bash
# First run: executes the command and caches results
shmemo echo "Hello"

# Second run: cache hit, output is replayed instantly
shmemo echo "Hello"
```

### Verbose mode

`-v` increases verbosity. Pass it up to three times for more detail:

```bash
shmemo -v ls -la       # -v:   cache hit/miss, purge messages
shmemo -vv ls -la      # -vv:  digest, temp dir operations
shmemo -vvv ls -la     # -vvv: environment variable capture, etc.
```

Verbose output goes to stderr.

### Quiet mode

```bash
shmemo -q some-command
```

Suppresses all `shmemo` messages, including errors. Conflicts with `-v`.

### TTL (time-to-live)

Cache entries are permanent by default. Use `--ttl` to expire them automatically:

```bash
shmemo --ttl 1h curl https://api.example.com/data
shmemo --ttl 30m expensive-script
shmemo --ttl 10s flaky-command
```

Expired entries are treated as cache misses and re-executed.

### Including environment variables in the cache key

By default, environment variables are **not** part of the cache key. Use `-e` to
include specific variables:

```bash
shmemo -e MY_VAR some-command
shmemo -e VAR1,VAR2 some-command
```

Different values of `MY_VAR` will produce separate cache entries.

### Purging the cache

```bash
shmemo --purge
```

Removes the entire cache directory. Mutually exclusive with running a command.

### Passing flags to the underlying command

If the underlying command has flags that look like `shmemo` flags, use `--` to end
`shmemo` option processing:

```bash
shmemo -- echo --verbose
```

### Complex commands

Remember: `shmemo` executes a process directly; it does not invoke a shell unless
you do.

```bash
shmemo sh -c 'echo out; echo err >&2; exit 42'
```

## How caching works

### Cache key

A cache entry is keyed by **SHA-256(argv + env)**:

- arguments are encoded in a canonical format (so `['a b']` differs from
  `['a','b']`)
- only environment variables explicitly listed via `-e` are included; the working
  directory is **not** part of the key

### Cache location

Cache directory:

- `$XDG_CACHE_HOME/shmemo/` if `XDG_CACHE_HOME` is set
- otherwise `~/.cache/shmemo/`

### On-disk layout

Each cached command is stored in a directory named by its digest:

```text
<cache_dir>/
  <digest>/
    meta.json
    stdout
    stderr
```

`stdout`/`stderr` are stored as raw bytes (binary-safe).

`meta.json` records the command, selected environment variables, exit code,
timestamp, digest, and optional expiry time.

### Concurrency

Concurrent cache misses for the same digest are handled without locks:

- each process writes to its own temp directory
  (`<digest>.tmp.<pid>.<timestamp>`)
- then atomically renames into place
- the first one wins; the rest clean up their temp directories
- orphaned temp directories older than 24 hours are removed at startup

## Environment variables

- `SHMEMO_DISABLE=1` — bypass caching and execute the command directly.
- `XDG_CACHE_HOME` — controls where cached results are stored.

## Security / safety

Shmemo writes command output to disk **unencrypted**. Do not use it with commands
that print sensitive data (tokens, credentials, private keys, PII, etc.).

On \*nix, the cache directory is created with `0700` permissions and output files
with `0600` (owner-only).

## Limitations

- No built-in cache pruning based on size or access time (LRU, etc.)
- The cache key includes `argv` and any explicitly selected environment variables;
  it does not automatically incorporate the working directory or the full process
  environment
- No cache format versioning: if the on-disk format changes, old entries become
  unreadable without a migration path
