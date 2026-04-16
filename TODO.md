# TODO

## Features

- [x] Expand `verbose` flag to support multiple levels
  - e.g. `-v` for info, `-vv` for debug, `-vvv` for trace
  - Add a unified logging system to replace `eprintln!` calls
- [ ] Argument to suppress all `memo` messages (even errors)
  - e.g. `--quiet`
- [ ] Argument to consider environment variables in cache key
  - Variables must be explicitly listed by user
  - e.g. `--env VAR1,VAR2,VAR3`
  - Short form: `-e VAR1,VAR2,VAR3`
  - By default, only `PWD` is considered
  - This supercedes the current behavior of always including working directory
- [ ] Argument to configure output capture
  - By default, capture stdout and stderr
  - e.g. `--capture stdout` to capture only stdout
  - e.g. `--capture stderr` to capture stderr only
  - e.g. `--capture stdout,stderr` to capture both stdout and stderr (default)
  - e.g. `--no-capture` to disable output capturing
- [ ] Support for capturing and caching arbitrary file descriptors
  - e.g. `--capture stdout,3,4` to capture stdout and file descriptors 3 and 4
- [ ] Argument to configure exit code capture
  - By default, capture exit code
  - e.g. `--no-exit-code` to disable exit code capturing
- [ ] Argument to set cache storage location
  - e.g. `--cache-dir /path/to/cache`
  - Short form: `-c /path/to/cache`
  - Default to standard cache directory
    - `$XDG_CACHE_HOME/memo`
    - Fallback to `$HOME/.cache/memo`
- [ ] Argument to set cache entry TTL (time-to-live)
  - e.g. `--ttl 1h` to set cache expiration to 1 hour
  - Support human-readable formats like `1h`, `30m`, `1d`
  - Default TTL is infinite (no expiration)
  - If it exists, store expiration time in metadata alongside cached entry
  - On retrieval, if entry has expiration and is expired, treat as cache miss
- [ ] Argument to evict existing entry
  - e.g. `--evict` to remove existing cache entry for the given command
- [ ] Argument to purge all cache entries
  - e.g. `--purge` to clear the entire cache
  - Not to be used with command to cache; only purges cache
- [ ] Configuration file support
  - e.g. `--config /path/to/config.toml`
  - Support setting default values for all command-line arguments
  - Use standard locations for config file if not specified
    - `$XDG_CONFIG_HOME/memo/config.toml`
    - Fallback to `$HOME/.config/memo/config.toml`
- [ ] Support for different hashing algorithms
  - e.g. `--hash sha256` to use SHA-256 instead of default
- [ ] Limit cache size
  - e.g. `--max-size 100MB` to limit cache size to 100 megabytes
  - Implement eviction policy (e.g. LRU) when limit is reached

## Build and Release

- [ ] Set up CI/CD pipeline for automated testing and releases
  - Use GitHub Actions or similar service
  - Enforce code quality checks (linting, formatting)
  - Automate release process on new tags
  - Tag determines version number (i.e. Cargo.toml version field)
