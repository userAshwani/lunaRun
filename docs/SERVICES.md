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
   │               Error ◄──── retries exhausted ─ Crashed ──(auto-restart ≤3 in 60s)──► Starting
   │                                                  
   └──────── Stopping ◄──────── stop() ───────── Running
```

- **Ready check** per type: TCP connect to the port (PHP, Caddy); for MySQL/MariaDB `mysqladmin --protocol=TCP -P <port> ping`. Timeout 30 s (DB first-run init: 120 s).
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

- **Windows:** every child is assigned to a **Job Object** with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so if lunaRun crashes, the OS kills the children. (A crashed DB then does InnoDB crash recovery on next start. That is acceptable and documented.)
- **macOS/Linux:** children run in their own process group. On normal exit, signal the group. If the app crashes, children may survive, so:
- **PID files** in `LUNA_HOME/run/<service-id>.pid` hold `{pid, exe, startedAt, port}`. On launch, for each PID file: if the process is alive **and** its executable path matches → offer **Adopt** (track it, health-check it) or **Stop**. Otherwise delete the stale PID file.

### Logs (FR-SV-05)

- stdout/stderr of each child → `LUNA_HOME/logs/<service-id>.log` (rotating 5 MB × 3) and a ring buffer of the last 2,000 lines in memory for the UI.
- Also tail the service's own log files (MySQL error log, Caddy access/error logs) into the same viewer as separate tabs.
- UI events are batched (≤ 10 per second per service).

## 3. Ports (FR-SV-03)

`PortAllocator` keeps assignments in `config/services.json` so they stay stable across restarts.

| Purpose | Range | Notes |
|---|---|---|
| Sites (MVP) | 8100–8999 | One per project, `http://localhost:<port>` |
| PHP FastCGI | 9100–9999 | Per pool; Windows uses N consecutive ports |
| MySQL/MariaDB | 3306, then 3307–3399 | First DB line gets 3306 if free |
| Caddy admin | 2119 | Not 2019 (the Caddy default), to avoid clashing with a user's own Caddy |
| Sites Phase 2 | 80 / 443 | `.test` domains, host-based routing on one Caddy listener |

Before every start: test-bind `127.0.0.1:<port>`. If it is in use, find the owner
(Windows: `GetExtendedTcpTable` → PID → process image path; macOS: `lsof -nP -iTCP:<port> -sTCP:LISTEN`; Linux: `/proc/net/tcp` + `/proc/<pid>/exe`)
and return `PORT_IN_USE { port, pid?, processName?, processPath? }`. The UI offers "Use port X instead"
or explains, for example, "XAMPP MySQL is running from C:\xampp\mysql\bin\mysqld.exe".

## 4. Config generation

Templates are embedded in the binary (`include_str!`) and rendered with `minijinja`. Generated files are
**owned by lunaRun and overwritten** on every relevant change. User customisation goes into
separate override files that lunaRun **never overwrites**.

### php.ini

- Base: the version's own `php.ini-development` (shipped in the Windows zips), or our template for static builds.
- lunaRun sets: `extension_dir` (absolute, quoted), Laravel extensions enabled, `memory_limit=512M`,
  `upload_max_filesize=128M`, `post_max_size=128M`, `max_execution_time=120`, `date.timezone` = OS timezone,
  `opcache.enable=1`, `opcache.validate_timestamps=1`, `opcache.revalidate_freq=0`, `curl.cainfo`/`openssl.cafile` → a CA bundle we ship
  (Windows PHP has no CA store, without this Composer/HTTP calls fail with SSL errors).
- Overrides: `config/php/<ver>/conf.d/99-user.ini`, loaded through the `PHP_INI_SCAN_DIR` env var on every PHP process we spawn (services and terminal).

### php-fpm.conf (macOS/Linux)

One pool, `listen = 127.0.0.1:<port>`, `pm = dynamic`, `pm.max_children = 8`, `catch_workers_output = yes`,
`clear_env = no`, error log → `logs/`.

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
MariaDB: `mariadb-install-db --datadir=…` (Windows: `mariadb-install-db.exe` / `mysql_install_db.exe`, depending on version).
Root has an empty password, which the UI states clearly ("local development only, bound to 127.0.0.1").

### Caddyfile (MVP)

```caddyfile
{
	admin 127.0.0.1:2119
	auto_https off
	persist_config off
}

# one block per running site
http://localhost:8101 {
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
