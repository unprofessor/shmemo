# memo

*memo* is a small program that memoizes (caches) shell command executions.

Run a command through `memo` once and it records:

- `stdout`
- `stderr`
- exit code

Then, when you run the same command again from the same working directory,
`memo` instantly replays the cached output instead of re-running the command.

## Why

Useful for commands that are expensive, flaky, rate-limited, or slow (e.g.
network calls), when you want fast repeatable runs during development.

## Install

### From source (recommended)

```bash
cargo install --path . --locked
```

### Build a release binary

```bash
cargo build --release
./target/release/memo --help
```

### Using GNU Guix

To build with a reproducible environment using [GNU Guix](https://guix.gnu.org/):

```bash
# Build the package
guix time-machine -C channels.scm -- build -f guix.scm

# Enter a development shell
guix time-machine -C channels.scm -- shell -m manifest.scm
```

## Usage

### Basic

```bash
# First run: executes the command and caches results
memo echo "Hello"

# Second run: cache hit, output is replayed
memo echo "Hello"
```

### Verbose mode

```bash
memo -v ls -la
```

Verbose output goes to stderr and shows hits/misses, the computed digest, and
other information.

### Passing flags to the underlying command

If the underlying command has flags that look like `memo` flags, use `--` to end
`memo` option processing:

```bash
memo -- echo --verbose
```

### Complex commands

Remember: `memo` executes a process directly; it does not invoke a shell unless
you do.

```bash
memo sh -c 'echo out; echo err >&2; exit 42'
```

## How caching works

### Cache key

A cache entry is keyed by **SHA-256(argv + cwd)**:

- arguments are encoded in a canonical format (so `['a b']` differs from
  `['a','b']`)
- current working directory is included so the same command in different
  directories gets different entries

### Cache location

Cache directory:

- `$XDG_CACHE_HOME/memo/` if `XDG_CACHE_HOME` is set
- otherwise `~/.cache/memo/`

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

### Concurrency

Concurrent cache misses for the same digest are handled without locks:

- each process writes to its own temp directory
  (`<digest>.tmp.<pid>.<timestamp>`)
- then atomically renames into place
- the first one wins; the rest clean up their temp directories

## Environment variables

- `MEMO_DISABLE=1` — bypass caching and execute the command directly.
- `XDG_CACHE_HOME` — controls where cached results are stored.

## Security / safety

Memo writes command output to disk **unencrypted**. Do not use it with commands
that print sensitive data (tokens, credentials, private keys, PII, etc.).

On \*nix, the cache directory and output files are created with restrictive
permissions (owner-only).

## Limitations

- No TTL/expiration policy
- No built-in cache pruning or "clear" subcommand (you can delete the cache
  directory manually)
- The cache key includes `argv` and `cwd`; it does not currently incorporate the
  full process environment
