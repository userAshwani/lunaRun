# lunaRun — Services, Process Supervision & Config

Related: [PRD §7.2, §7.4](PRD.md#72-services) · [ARCHITECTURE](ARCHITECTURE.md)

## 1. Service types

| Service id pattern | Started from | Process | Listens on |
|---|---|---|---|
| `php-fcgi@<version>` (Windows) | PHP runtime | N × `php-cgi.exe -b 127.0.0.1:<port_i> -c <php.ini>` (default N = 4, configurable) | `127.0.0.1:<port>…<port+N-1>` |
| `php-fpm@<version>` (macOS/Linux) | PHP runtime | `php-fpm --nodaemonize --fpm-config <conf> -c <php.ini>` | `127.0.0.1:<port>` |
| `mysql@<line>` | MySQL runtime | `mysqld --defaults-file=<my.cnf>` (console mode on Windows) | `127.0.0.1:<port>` (default 3306, next 3307…) |
| `mariadb@<line>` | MariaDB runtime | `mariadbd --defaults-file=<my.cnf>` | same scheme |
| `caddy` | Caddy runtime | `caddy run --config <Caddyfile> --adapter caddyfile` | site ports; admin API `127.0.0.1:2119` |
| `worker@<project>/<name>` (Phase 3) | project | `php artisan queue:work`, `npm run dev`, … | — |

On Windows each `php-cgi` of a pool is a service of its own, `php-fcgi@<version>-w<n>` (D-028): the supervisor owns one process per service.

**PHP pools start on demand**: a pool runs only while at least one running site is bound to that version.
Several PHP versions can run at the same time (FR-SV-02). A pool is identified by its **exact** version.
A project bound to line `8.3` uses the highest installed 8.3.x when it starts.

DB services are identified by **engine + line** (`mysql@8.4`): the data dir belongs to the line, and
patch upgrades within a line reuse it (MySQL runs its own in-place upgrade on start).

## 2. Supervisor

One `Supervisor` (tokio task) owns all children. The UI and tray send it commands, and it emits state events.

### State machine

```
         start()                ready check ok
Stopped ────────► Starting ──────────────────► Running
   ▲                 │ timeout / exit             │  exit (unexpected)
   │                 ▼                            ▼
   │               Error                         Crashed ──(auto-restart ≤3 in 60s)──► Starting
   │                                                  
   └──────── Stopping ◄──────── stop() ───────── Running
```

- **`Error` vs `Crashed`** (as built in `luna-core::services`, D-023): `Error` is a start that never worked (the program
  could not be launched, it exited before it was ready, or it was not ready in time). It is not retried, because a bad
  configuration or a taken port will not fix itself. `Crashed` is a process that died while running. When the automatic
  restarts run out the service **stays `Crashed`** with `gave_up` set (FR-SV-06), it does not become `Error`. A restart that
  fails before it is ready counts as one more crash. Stopping a `Crashed` or `Error` service makes it `Stopped`.
- **Ready check** per type: TCP connect to the port (PHP, Caddy); for MySQL/MariaDB `mysqladmin --protocol=TCP -P <port> ping`. Timeout 30 s (DB first-run init: 120 s). The TCP check retries every 100 ms with a 100 ms connect timeout: it is always loopback, and on Windows a connection to a port that is not open yet takes about 500 ms to be refused, so a longer timeout would notice a service that is up half a second late.
- **Restart policy** (FR-SV-06): unexpected exit → wait 1 s, 2 s, 4 s → after 3 failures in 60 s → `Crashed`, plus a desktop notification.
- **Windows php-cgi** exits by design after `PHP_FCGI_MAX_REQUESTS` (set to 500). That is a normal recycle: restart the worker straight away without counting it as a crash.

### Graceful stop

| Service | Stop method | Force after |
|---|---|---|
| MySQL / MariaDB | `mysqladmin -u root -P <port> --protocol=TCP shutdown` | 30 s → kill |
| php-fpm | SIGQUIT (graceful) | 10 s → SIGKILL |
| php-cgi (Windows) | terminate (stateless) | — |
| Caddy | `caddy stop --address 127.0.0.1:2119`, or the admin API `/stop` | 10 s → kill |

On app **Quit**: stop sites → PHP pools → DBs, in parallel within each group, and show a progress window.

### No orphans (FR-SV-07)

As built in `luna-core::services` (D-024):

- **Windows:** every service gets its own **Job Object** with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so if lunaRun crashes the OS kills the children. When a service ends normally its job is closed, which kills what it left behind. (A crashed DB then does InnoDB crash recovery on next start. That is acceptable and documented.)
- **macOS/Linux:** every service runs in its **own process group**, and when it ends the whole group is killed (`kill(-pgid, SIGKILL)`). If lunaRun crashes, the children survive, so:
- **PID files** in `LUNA_HOME/run/<service-id>.pid` hold `{pid, exe, startedAt, port}` (written atomically, with a `schema` field). They are written when the process starts and removed when it ends. On launch, `Supervisor::scan_leftovers` looks at each file: if the process is alive **and** its executable path matches, it is a leftover and the UI offers **Adopt** (`adopt(spec)`: tracked again with the normal lifecycle and restart policy, after the ready check passes) or **Stop** (`stop_leftover(id)`: ask it to leave, kill it after 10 s). Otherwise the file is stale and is deleted.

### Logs (FR-SV-05)

As built in `luna-core::services` (D-025):

- stdout and stderr of each child are **one file**, `LUNA_HOME/logs/<service-id>.log`, which the child writes to directly (so it survives a crash of lunaRun and an adopted service keeps writing to it). lunaRun tails it: a ring of the last 2,000 lines in memory per source, and `Supervisor::log_lines(id, source, max)` for the viewer to fill itself. The source of the service's own output is `output`.
- Rotation (5 MB x 3 files): a new run renames the old files up by one; a running service is copied aside and truncated at 5 MB. An automatic restart or an adoption does not rotate.
- Also tail the service's own log files (MySQL error log, Caddy access/error logs) into the same viewer as separate tabs: `ServiceSpec::log_files`, each with a label, inside `LUNA_HOME`.
- UI events are batched: one `service:log` event per tick per service, at most 10 per second, carrying every new line of every source.
- lunaRun adds `[lunaRun] ...` lines to the log (starting, exited with code N, stopped).

## 3. Ports (FR-SV-03)

`PortAllocator` keeps assignments in `config/services.json` so they stay stable across restarts.

| Purpose | Range | Notes |
|---|---|---|
| Sites (MVP) | 8100–8999 | One per project, `http://localhost:<port>` |
| PHP FastCGI | 9100–9999 | Per pool; Windows uses N consecutive ports |
| MySQL/MariaDB | 3306, then 3307–3399 | First DB line gets 3306 if free |
| Caddy admin | 2119 | Not 2019 (the Caddy default), to avoid clashing with a user's own Caddy |
| Sites Phase 2 | 80 / 443 | `.test` domains, host-based routing on one Caddy listener |

As built in `luna-core::services` (D-026): `PortAllocator::assign(id, purpose)` gives a service its ports and saves them,
`reassign` gives it others ("Use port X instead"), `release` frees them. Before every start someone asked for,
`Supervisor::start` checks `ServiceSpec::ports` (`check_ports`): a port is taken if a test bind of `127.0.0.1:<port>` fails **or** the OS
lists a listening socket on it that would clash (on Windows and macOS the bind alone misses a program that listens on the
wildcard address, as XAMPP's `mysqld` does). Who holds it comes from `GetExtendedTcpTable` (Windows), `/proc/net/tcp` and
`/proc/<pid>/fd` (Linux), `lsof -nP -iTCP -sTCP:LISTEN -Fpcn` (macOS), then the program's path. The error is
`PORT_IN_USE { port, pid?, processName?, processPath? }`. The UI offers "Use port X instead"
or explains, for example, "XAMPP MySQL is running from C:\xampp\mysql\bin\mysqld.exe". A full range is `NO_FREE_PORT`.

## 4. Config generation

Templates are embedded in the binary (`include_str!`) and filled with plain string replacement (D-020: `minijinja` turned out not to be needed for `php.ini`; it may still come for the Caddyfile or `my.cnf`). Generated files are
**owned by lunaRun and overwritten** on every relevant change. User customisation goes into
separate override files that lunaRun **never overwrites**.

### php.ini

- Base: the version's own `php.ini-development` (shipped in the Windows zips), edited in place so only the lines below change, or our template for static builds (`luna-core::runtimes::php_config`, D-020).
- lunaRun sets: `extension_dir` (absolute, quoted), Laravel extensions enabled, `memory_limit=512M`,
  `upload_max_filesize=128M`, `post_max_size=128M`, `max_execution_time=120`, `date.timezone` = OS timezone,
  `opcache.enable=1`, `opcache.validate_timestamps=1`, `opcache.revalidate_freq=0`, `curl.cainfo`/`openssl.cafile` → a CA bundle we ship
  (Windows PHP has no CA store, without this Composer/HTTP calls fail with SSL errors).
- Overrides: `config/php/<ver>/conf.d/99-user.ini`, loaded through the `PHP_INI_SCAN_DIR` env var on every PHP process we spawn (services and terminal). `php_environment(home, version)` returns `PHPRC` and `PHP_INI_SCAN_DIR` for that; use it for every PHP process.

### php-fpm.conf (macOS/Linux)

One pool, `listen = 127.0.0.1:<port>`, `pm = dynamic`, `pm.max_children = 8`, `catch_workers_output = yes`,
`clear_env = no`, error log → `logs/`.

**As built** (`luna_core::services::php`, D-028): `php_definition(home, ports, runtime, os, workers)` returns the specs, the
FastCGI ports and the pool id. The file is `config/php/<version>/php-fpm.conf`, rewritten on every definition, with
`daemonize = no` and the error log in `logs/php-fpm-<version>.log` (the tab `PHP-FPM log`). On Windows there is no such
file: each `php-cgi.exe -b 127.0.0.1:<port> -c <php.ini>` is its own service.

### my.cnf / my.ini

```ini
[mysqld]
basedir = "<runtime dir>"
datadir = "<LUNA_HOME>/data/mysql-8.4"
port = 3306
bind-address = 127.0.0.1
socket = "<LUNA_HOME>/run/mysql-8.4.sock"       # unix only
log-error = "<LUNA_HOME>/logs/mysql-8.4.err"
character-set-server = utf8mb4
collation-server = utf8mb4_unicode_ci
max_allowed_packet = 64M
# user overrides: !include <LUNA_HOME>/config/mysql/8.4/user.cnf
```

**First start** (FR-SV-10): if the data dir is empty → MySQL `mysqld --defaults-file=… --initialize-insecure`;
MariaDB: `mariadb-install-db --datadir=…` (Windows) or with `--defaults-file`, `--basedir`, `--auth-root-authentication-method=normal`, `--skip-test-db` (Unix).
Root has an empty password, which the UI states clearly ("local development only, bound to 127.0.0.1").

**As built** (`luna_core::services::databases`, D-027): the service is `mysql@<line>` / `mariadb@<line>` and the template
above is `templates/my.ini` plus `mysqlx = OFF` for MySQL 8 and later and, on Unix, the socket. Setup is a separate step,
`DatabaseDefinition::initialise`, run before `Supervisor::start` so it can show progress and be cancelled; it removes the
data folder when it fails. Ready check and shutdown are `mysqladmin`/`mariadb-admin` `ping` and `shutdown` over TCP.

### Caddyfile (MVP)

```caddyfile
{
	admin 127.0.0.1:2119
	auto_https off
	persist_config off
}

# one block per running site
http://localhost:8101, http://127.0.0.1:8101 {
	bind 127.0.0.1
	root * "D:/Work/shop/public"
	encode gzip
	php_fastcgi 127.0.0.1:9101 127.0.0.1:9102 127.0.0.1:9103 127.0.0.1:9104
	file_server
	log {
		output file "<LUNA_HOME>/logs/sites/shop.access.log"
	}
}
```

- Windows paths use forward slashes and are quoted.
- Changing a site's PHP version or adding/removing a site → re-render, then `caddy reload --config … --address 127.0.0.1:2119`. No restart, other sites stay up (FR-ST-03).
- Validate with `caddy validate` before reloading. On failure keep the old config and show the error.

**As built** (`luna_core::services::caddy`, D-028): `caddy_definition(home, ports, runtime, sites, os)` writes
`config/caddy/Caddyfile` (rewritten every time) and returns the spec (ready when the first site answers, stopped with
`caddy stop --address`, every site port checked before a start, one access-log tab per site, Caddy's state inside
LUNA_HOME through `XDG_DATA_HOME` and `XDG_CONFIG_HOME`). `CaddyDefinition::validate` and `reload` run the two commands
above with their output kept in `logs/caddy.check.log` (`CONFIG_INVALID`, `RELOAD_FAILED`). `bind 127.0.0.1` and the second
host name `127.0.0.1` are additions to the template above: a site is not offered to the network, and a block for the
name `localhost` alone would not answer a request for `127.0.0.1`. A project folder is only written down as text and a
name with braces (Caddy's placeholders) is refused.

**Phase 2:** host-based blocks `shop.test { … }` on :80/:443 with `tls internal` (Caddy's local CA,
trusted once via `caddy trust` with elevation), plus hosts-file entries managed between
`# BEGIN lunaRun` / `# END lunaRun` markers.

## 5. Terminal with project environment (FR-PJ-08)

Build the env for the project:
`PATH = <php dir> ; <composer shim dir> ; <node dir> ; <original PATH>`, `PHP_INI_SCAN_DIR = <conf.d>`,
`LUNA_PROJECT = <id>`. A per-version `composer` shim (`composer.bat` on Windows / a shell script elsewhere)
runs `php composer.phar "$@"` with the bound PHP.

| OS | Launch |
|---|---|
| Windows | Windows Terminal (`wt.exe -d <dir>`) if present, else PowerShell / cmd, env passed to the child process |
| macOS | Write a temporary `.command` script that exports the env and `cd`s, then `open -a Terminal` (or iTerm if set in settings) |
| Linux | First found of `x-terminal-emulator`, `gnome-terminal`, `konsole`, `xterm` with the env set |

No global or user `PATH` changes in Phase 1 (shims come in Phase 2, opt-in, FR-SH-02).
