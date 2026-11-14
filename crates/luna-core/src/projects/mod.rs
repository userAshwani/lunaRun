//! What a project folder needs, and which runtime fits (FR-PJ-01 to FR-PJ-07; docs/PROJECT-DETECTION.md; D-035).
//!
//! Two pure functions and nothing else:
//!
//! - [`detect`] reads a project folder and says what it asks for: the PHP and Node requirements with where each came
//!   from, the PHP extensions, the database it expects, the framework and the document root. It **only reads** (FR-PJ-05,
//!   golden rule 3): it never writes into the folder, never runs project code, `composer` or `npm`. It **never fails**: a
//!   missing or broken file is a [`Warning`] and detection goes on.
//! - [`resolve`] takes that report, what is installed and what the registry offers, and says which version to use
//!   ([`Resolution`]): the highest installed one that fits, else the one to install. That nothing fits is an answer
//!   ([`ResolutionIssue`]), not an error.
//!
//! [`ProjectStore`] keeps the list of projects the user added (their folder, name, document root and manual version
//! choices) in `config/projects.json` and calls these two for the report; it never keeps the report itself.

mod data;
mod detect;
mod dotenv;
mod files;
mod resolve;
mod store;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::constraints::VersionReq;

pub use data::DetectionData;
pub use detect::detect;
pub use resolve::{ResolvePrefs, resolve};
pub use store::{
    BindingChoice, Bindings, DocRootChoice, LastDetection, Project, ProjectDetection, ProjectPatch,
    ProjectStore, Surroundings,
};

/// How much of the data the page needs to tell apart: whether a source is an exact pin, a constraint or a table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Selector {
    /// A version constraint: `constraint` holds it.
    Constraint,
    /// `lts/*`: the newest long-term-support line.
    LatestLts,
    /// `node`, `latest`: the newest version.
    Latest,
}

/// Where a piece of a requirement was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SourceKind {
    PhpVersionFile,
    ComposerPlatformConfig,
    ComposerRequire,
    ComposerLock,
    ComposerLockPlatform,
    FrameworkTable,
    NvmRc,
    NodeVersionFile,
    Volta,
    Engines,
}

/// One input of a requirement, kept so the page can show `matches ^8.2 (composer.json)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Source {
    pub kind: SourceKind,
    /// The file, relative to the project folder (`composer.json`), or the data file for a table row.
    pub file: String,
    /// Where in the file (`require.php`, `packages[laravel/framework].require.php`); empty for a whole-file source.
    pub path: String,
    /// The text as written (`^8.2`).
    pub raw: String,
    /// What it means, as lunaRun shows constraints (`>=8.2.0 <9.0.0`).
    pub constraint: String,
}

/// A version the project is pinned to: it replaces what the files compute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Pin {
    /// As written (`8.3`, `8.3.14`, `20`, `lts/iron`).
    pub text: String,
    /// A whole version (three numbers): a binding to it is exact, not to its line.
    pub exact: bool,
    pub file: String,
}

/// The version line a team says it targets (`config.platform.php`): preferred when it fits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Preferred {
    pub text: String,
    /// `8.2` for `8.2.0`.
    pub line: String,
    pub file: String,
    pub path: String,
}

/// What a project asks of one runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Requirement {
    /// The effective requirement as text (`>=8.2.0 <9.0.0`, `lts/*`).
    pub constraint: String,
    /// The same, as something to match versions against. Not sent to the page: `constraint` is its text.
    #[serde(skip)]
    #[ts(skip)]
    pub req: VersionReq,
    pub selector: Selector,
    /// Set when a pin replaced what the project files compute.
    pub pin: Option<Pin>,
    /// What the files compute, shown next to a pin that replaced it.
    pub computed: Option<String>,
    pub preferred: Option<Preferred>,
    pub sources: Vec<Source>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FrameworkKind {
    Laravel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Framework {
    pub kind: FrameworkKind,
    /// From `composer.lock` (`12.3.0`), when it says.
    pub version: Option<String>,
    /// What made it Laravel, in words (`artisan`, `laravel/framework in composer.lock`).
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocRoot {
    /// Relative to the project folder, with `/`: `public`, or `.` for the folder itself.
    pub path: String,
    pub evidence: String,
}

/// What lunaRun's own PHP has of an extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Availability {
    /// Loaded.
    Enabled,
    /// Shipped as a loadable extension that lunaRun does not switch on.
    Disabled,
    /// Not in the build.
    Absent,
    /// The data does not know this extension: not a statement that it is missing.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtRequirement {
    /// The Composer name without `ext-`, lower case (`pdo_mysql`, `zend-opcache`).
    pub name: String,
    /// The name in php.ini (`opcache` for `zend-opcache`).
    pub ini_name: String,
    /// Loaded with `zend_extension=`.
    pub zend_extension: bool,
    /// Who asks for it: `composer.json`, a package name, or `Laravel` for its baseline. Sorted.
    pub needed_by: Vec<String>,
    /// What the PHP lunaRun installs on Windows has of it.
    pub windows: Availability,
    /// What the static PHP lunaRun installs on macOS and Linux has of it.
    pub unix: Availability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DbEngine {
    Mysql,
    Mariadb,
    Pgsql,
    Sqlite,
    Sqlsrv,
    /// `DB_CONNECTION` is missing or not one lunaRun knows.
    Unknown,
}

/// What `.env` says about the database. **Never the password**: that key is not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DbHint {
    pub engine: DbEngine,
    /// `DB_CONNECTION` as written.
    pub connection: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub username: Option<String>,
    /// `.env`, or `.env.example` when there is no `.env`.
    pub file: String,
}

/// Something else the project seems to use (Phase 2: nothing starts it yet).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServiceHint {
    /// `redis`, `mail`.
    pub service: String,
    /// The setting that says so, with its value when that is not a secret (`CACHE_STORE=redis`).
    pub reason: String,
}

/// Stable codes the page turns into text, as for errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export)]
pub enum WarningCode {
    /// The project folder does not exist or is not a folder.
    ProjectFolderMissing,
    /// A file that should be JSON is not.
    InvalidJson,
    /// A file is too large to read (it is ignored).
    FileTooLarge,
    /// A file exists and cannot be read (permissions, in use).
    UnreadableFile,
    /// A version constraint cannot be read (it is ignored).
    InvalidConstraint,
    /// A constraint names a development branch: it does not restrict the version.
    DevBranch,
    /// `.php-version` is not a version.
    InvalidPhpVersionFile,
    /// `.nvmrc`, `.node-version` or `volta.node` is not a form lunaRun knows.
    UnsupportedNodeVersion,
    /// `lts/<name>` is not a codename lunaRun knows.
    UnknownLtsCodename,
    /// The pinned PHP version lies outside what the Composer files allow.
    PinConflictsWithComposer,
    /// The pinned Node version lies outside `engines.node`.
    PinConflictsWithEngines,
    /// The PHP requirements of the Composer files cannot all be met by any version.
    UnsatisfiablePhpConstraint,
    /// Laravel is there but its version is not (no lock file).
    FrameworkVersionUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Warning {
    pub code: WarningCode,
    pub file: Option<String>,
    /// For a log or a bug report, in English. The page shows its own text for the `code`.
    pub message: String,
    /// What the message is about: the packages that conflict, the raw constraint, the pin.
    pub details: Vec<String>,
}

/// Everything [`detect`] found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DetectionReport {
    pub framework: Option<Framework>,
    pub doc_root: DocRoot,
    pub php: Option<Requirement>,
    pub php_extensions: Vec<ExtRequirement>,
    pub node: Option<Requirement>,
    /// `pnpm@9.1.0`, from `package.json`: information only (Corepack, Phase 2).
    pub package_manager: Option<String>,
    pub database: Option<DbHint>,
    pub services_hints: Vec<ServiceHint>,
    pub warnings: Vec<Warning>,
    /// SHA-256 of every input file (and of which marker files exist): changes when the project's inputs do.
    pub inputs_hash: String,
}

/// How a binding is made: to a version line (`8.3`, follows its patch versions) or to an exact version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BindingMode {
    Line,
    Exact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Binding {
    pub mode: BindingMode,
    /// `8.3` or `8.3.14`.
    pub value: String,
}

/// The version to use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Recommendation {
    pub version: String,
    pub line: String,
    /// It is installed. When it is not, `needs_install` says it has to be fetched first.
    pub installed: bool,
    pub needs_install: bool,
    /// The line has reached its end of life.
    pub eol: bool,
    /// The download is an Intel build that runs under Rosetta on this Mac.
    pub rosetta_fallback: bool,
    pub binding: Binding,
}

/// Why a requirement was resolved the way it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Basis {
    /// The user chose it for this project.
    Manual,
    /// A pin in the project (`.php-version`, `.nvmrc`, `volta`).
    Pinned,
    /// The constraint the project files compute.
    Constraint,
    /// A row of the framework table (nothing else said).
    FrameworkTable,
    /// The project says nothing: the user's default, else the newest.
    NoRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[ts(export)]
pub enum IssueCode {
    /// No version of the runtime satisfies the requirement on this computer.
    NoCompatibleVersion,
    /// The requirement itself is empty: nothing could satisfy it, anywhere.
    UnsatisfiableConstraint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResolutionIssue {
    pub code: IssueCode,
    /// In English, for a log: why. The page shows its own text for the `code`.
    pub message: String,
    /// Version lines that satisfy the requirement but have no download for this computer (`7.4`).
    pub unavailable_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuntimeResolution {
    /// `php` or `node`.
    pub runtime: String,
    pub basis: Basis,
    /// What was matched, as text; none when the project says nothing.
    pub constraint: Option<String>,
    pub recommended: Option<Recommendation>,
    /// Installed versions that satisfy it, newest first.
    pub installed_matches: Vec<String>,
    pub issues: Vec<ResolutionIssue>,
    /// Set when the user's own choice (`basis` is `manual`) does not satisfy what the project asks for (FR-PJ-04): that
    /// requirement as text, so the page can warn "this project asks for >=8.3.0 <9.0.0". Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub conflicts_with: Option<String>,
}

/// What lunaRun suggests for the database a project expects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DatabaseSuggestion {
    pub engine: DbEngine,
    /// The runtime to run (`mysql`, `mariadb`), or none: SQLite needs nothing, PostgreSQL and SQL Server are not supported.
    pub runtime: Option<String>,
    /// The line (`8.4`), when there is a runtime.
    pub line: Option<String>,
    /// The newest installed version of that line, if any.
    pub installed_version: Option<String>,
    pub note: DbNote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DbNote {
    None,
    /// MariaDB is not available on macOS: MySQL is suggested.
    MariadbNotOnMacos,
    /// SQLite: nothing to run, but `pdo_sqlite` must be there.
    SqliteNeedsNothing,
    /// PostgreSQL: not supported yet.
    PostgresNotSupported,
    /// SQL Server: not supported.
    SqlServerNotSupported,
    /// The project does not say which database.
    NoDatabaseSpecified,
}

/// What an extension the project needs comes to on the PHP that lunaRun would run it with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtensionStatus {
    pub name: String,
    pub ini_name: String,
    pub needed_by: Vec<String>,
    pub availability: Availability,
}

/// Everything [`resolve`] decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Resolution {
    pub php: RuntimeResolution,
    /// `None` when there is no `package.json`: Node is not required.
    pub node: Option<RuntimeResolution>,
    pub extensions: Vec<ExtensionStatus>,
    pub database: Option<DatabaseSuggestion>,
}
