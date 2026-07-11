//! The Capability Runtime — the single place in GX that authorizes access
//! to sensitive resources (filesystem, network, process execution, AI
//! providers, bridges, ...).
//!
//! This is deliberately framed as *capability*, not *permission*: a
//! permission check answers "can I execute this operation?" at the call
//! site where the operation happens. A capability answers "what resources
//! is this program allowed to access?" as a property of the running
//! program itself, decided in one place. Every dangerous runtime operation
//! asks the same question through the same function — [`Capabilities::authorize`]
//! — instead of each subsystem (shell, HTTP, bridges, AI, ...)
//! re-implementing its own gate. The subsystem's job is to classify *what*
//! it's about to do (which [`Resource`], which name within it) and execute
//! once authorized; deciding whether that's allowed is this module's job
//! alone.
//!
//! Deliberately excluded, per GX's own design philosophy: no policy engine
//! (no conditions, no rules language), no RBAC, no user identities. Grants
//! are static for the lifetime of one `Interpreter`, resolved once at
//! startup from CLI flags + `gx.json`, and checked with plain data — a
//! boolean, or a small allowlist of names. That's the entire model, and
//! it's meant to stay that way as new resource kinds are added.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

// ── Resource identity ──────────────────────────────────────────────────────────

/// Every distinct resource domain the Capability Runtime can gate.
///
/// Adding a new runtime subsystem (a future networking mode, a package
/// manager, a native plugin host, a distributed-runtime transport, ...)
/// means adding one variant here, one field on [`Capabilities`], one arm in
/// [`Capabilities::authorize`], and one string in [`Resource::name`]/
/// [`Resource::parse`] — nothing else in the runtime changes shape, because
/// every call site already goes through the same `authorize` entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resource {
    /// `shell()`/`exec()` — arbitrary shell string execution.
    Shell,
    /// `process_run`/`process_spawn` — structured, argument-array process execution.
    Process,
    /// Filesystem reads/writes (scope carried separately — see [`FilesystemAccess`]).
    Filesystem,
    /// Outbound HTTP/network requests to private/loopback/link-local addresses.
    InternalNetwork,
    /// Outbound HTTP/network requests to public addresses.
    ExternalNetwork,
    /// `serve on port N { ... }` — binding a listening HTTP server.
    HttpServer,
    /// `db_query`/`db_exec`/`db_transaction` — SQLite access.
    Database,
    /// `env`/`get_env`/`set_env` — host environment variable access.
    Environment,
    /// `ask <provider> { ... }`/`embed`/`infer classifier` — AI provider calls.
    AiProviders,
    /// `use js.X` / inline `js.module.method()` calls.
    JsBridge,
    /// `use ts.X`.
    TsBridge,
    /// `use py.X`.
    PyBridge,
    /// `use binary "path"`.
    BinaryBridge,
    /// `use go "path"`.
    GoBridge,
    /// `use rust_bin "path"`.
    RustBinBridge,
}

impl Resource {
    /// Stable string identity used in `gx.json`, `--deny <name>`, and error
    /// messages — one canonical name per resource, everywhere.
    pub fn name(&self) -> &'static str {
        match self {
            Resource::Shell => "shell",
            Resource::Process => "process",
            Resource::Filesystem => "filesystem",
            Resource::InternalNetwork => "internal_network",
            Resource::ExternalNetwork => "external_network",
            Resource::HttpServer => "http_server",
            Resource::Database => "database",
            Resource::Environment => "environment",
            Resource::AiProviders => "ai",
            Resource::JsBridge => "js",
            Resource::TsBridge => "ts",
            Resource::PyBridge => "py",
            Resource::BinaryBridge => "binary",
            Resource::GoBridge => "go",
            Resource::RustBinBridge => "rust_bin",
        }
    }

    pub fn parse(s: &str) -> Option<Resource> {
        Some(match s {
            "shell" => Resource::Shell,
            "process" => Resource::Process,
            "filesystem" => Resource::Filesystem,
            "internal_network" => Resource::InternalNetwork,
            "external_network" => Resource::ExternalNetwork,
            "http_server" => Resource::HttpServer,
            "database" => Resource::Database,
            "environment" => Resource::Environment,
            "ai" => Resource::AiProviders,
            "js" => Resource::JsBridge,
            "ts" => Resource::TsBridge,
            "py" => Resource::PyBridge,
            "binary" => Resource::BinaryBridge,
            "go" => Resource::GoBridge,
            "rust_bin" => Resource::RustBinBridge,
            _ => return None,
        })
    }
}

// ── Scoping types ─────────────────────────────────────────────────────────────

/// A name-scoped resource: either open to any name, or restricted to an
/// explicit set. Used for process executables, bridge modules/paths, and AI
/// providers — every place GX already had (or now gains) an allowlist.
#[derive(Debug, Clone)]
pub enum Allowlist {
    Any,
    Only(HashSet<String>),
}

impl Allowlist {
    pub fn only<I: IntoIterator<Item = String>>(names: I) -> Self {
        Allowlist::Only(names.into_iter().collect())
    }

    pub fn permits(&self, name: &str) -> bool {
        match self {
            Allowlist::Any => true,
            Allowlist::Only(set) => set.contains(name),
        }
    }
}

/// Filesystem access scope. A separate type (not a bool) because "granted"
/// isn't binary for the filesystem — it's granted-within-a-directory or
/// granted-everywhere.
#[derive(Debug, Clone)]
pub enum FilesystemAccess {
    Sandboxed(PathBuf),
    Unrestricted,
}

/// Environment-variable access scope: open, or open except for an explicit
/// denylist of exact variable names. Deliberately exact-match, not glob —
/// glob/pattern matching is a real feature with real edge cases (what does
/// `*_KEY` mean exactly?) that isn't needed to close the actual gap (`env()`
/// having no gate at all today); an exact denylist is simple, deterministic,
/// and auditable, matching the Capability Runtime's own design goals.
#[derive(Debug, Clone)]
pub enum EnvAccess {
    Unrestricted,
    Denied(HashSet<String>),
}

// ── Grants ────────────────────────────────────────────────────────────────────

/// The complete set of capability grants for one `Interpreter` — the single
/// source of truth every dangerous runtime operation is authorized against.
/// Cheap to clone (a handful of bools/enums + small string sets), which
/// matters because spawned agents and `parallel {}` branches each get their
/// own `Interpreter` and must inherit these, not reset to defaults.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub filesystem: FilesystemAccess,
    pub shell: bool,
    pub process: bool,
    pub process_executables: Allowlist,
    pub internal_network: bool,
    pub external_network: bool,
    pub http_server: bool,
    pub database: bool,
    pub environment: EnvAccess,
    pub ai_providers: Allowlist,
    pub js_modules: Allowlist,
    pub ts_modules: Allowlist,
    pub py_modules: Allowlist,
    pub binary_executables: Allowlist,
    pub go_executables: Allowlist,
    pub rust_bin_executables: Allowlist,
    /// Operator-level denials (`--deny <resource>`) — checked before
    /// anything else and cannot be overridden by `gx.json`. This is the one
    /// piece of the model that exists specifically so the entity invoking
    /// `gx` (a human operator, a deployment config) has the final say over
    /// the entity that wrote the script or its manifest — the same attacker
    /// who could modify a script can usually modify the `gx.json` sitting
    /// next to it, so manifest grants are a weaker trust boundary than an
    /// explicit, out-of-band CLI decision.
    denied: HashSet<Resource>,
}

/// Why an `authorize` call was refused — carries enough structure for the
/// interpreter layer to format a message consistent with GX's existing
/// "disabled by default, run with --flag" / "not listed in gx.json"
/// conventions, without this module needing to know about `Signal` or any
/// interpreter-specific error type.
#[derive(Debug, Clone)]
pub enum Denial {
    /// An operator explicitly denied this resource with `--deny`.
    OperatorDenied { resource: Resource },
    /// The resource itself isn't granted at all (e.g. `shell` without `--allow-shell`).
    NotGranted {
        resource: Resource,
        hint: &'static str,
    },
    /// The resource is granted, but this specific name isn't in its allowlist.
    NotInAllowlist { resource: Resource, name: String },
    /// This specific environment variable name is on the `env_deny` list.
    EnvVarDenied { name: String },
    /// A filesystem path resolved outside the sandbox directory.
    OutsideSandbox { path: PathBuf, sandbox: PathBuf },
}

impl std::fmt::Display for Denial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Denial::OperatorDenied { resource } => {
                write!(f, "'{}' was explicitly denied with --deny", resource.name())
            }
            Denial::NotGranted { resource, hint } => {
                write!(f, "'{}' is disabled by default. {}", resource.name(), hint)
            }
            Denial::NotInAllowlist { resource, name } => {
                write!(
                    f,
                    "'{}' is not listed in gx.json's allowlist for '{}'",
                    name,
                    resource.name()
                )
            }
            Denial::EnvVarDenied { name } => {
                write!(
                    f,
                    "environment variable '{}' is denied by gx.json's capabilities.env_deny",
                    name
                )
            }
            Denial::OutsideSandbox { path, sandbox } => {
                write!(
                    f,
                    "'{}' is outside the allowed directory '{}'",
                    path.display(),
                    sandbox.display()
                )
            }
        }
    }
}

impl Capabilities {
    /// The safe-by-default grant set — identical to what a bare
    /// `Interpreter::new()` produced before the Capability Runtime existed:
    /// shell/process/internal-network denied, filesystem unrestricted
    /// (callers that want sandboxing set `filesystem` explicitly, exactly
    /// as they previously set `sandbox_dir`), everything else that was
    /// unconditionally available (AI, environment, external network,
    /// http_server, database, every bridge) stays open. Closing any of
    /// these gaps is something a script/operator now *can* do via
    /// `gx.json`/`--deny` — not something this milestone silently does for
    /// them, which would break every existing GX program.
    pub fn new() -> Self {
        Capabilities {
            filesystem: FilesystemAccess::Unrestricted,
            shell: false,
            process: false,
            process_executables: Allowlist::Any,
            internal_network: false,
            external_network: true,
            http_server: true,
            database: true,
            environment: EnvAccess::Unrestricted,
            ai_providers: Allowlist::Any,
            js_modules: Allowlist::Any,
            ts_modules: Allowlist::Any,
            py_modules: Allowlist::Any,
            binary_executables: Allowlist::Any,
            go_executables: Allowlist::Any,
            rust_bin_executables: Allowlist::Any,
            denied: HashSet::new(),
        }
    }

    /// Record an operator-level denial. Always wins over every other grant.
    pub fn deny(&mut self, resource: Resource) {
        self.denied.insert(resource);
    }

    /// Apply a parsed `gx.json` manifest on top of the current grants.
    ///
    /// `dependencies.*` (js/ts/py/binary/go/rust_bin/process/ai) narrows an
    /// open-by-default allowlist to the declared names — the same
    /// "declared restricts, absent stays open" convention GX already used
    /// for `dependencies.js`/`dependencies.py`/`dependencies.process`,
    /// extended uniformly to every namespace instead of just those three.
    ///
    /// `capabilities.*` (`external_network`, `http_server`, `database`,
    /// `env_deny`) restricts the resources that aren't naturally a name
    /// list. Deliberately absent: `shell`, `process`, `internal_network`.
    /// Those three grant access to the most dangerous operations GX has
    /// (arbitrary shell strings, arbitrary executables, and requests to
    /// private network addresses), and `gx.json` ships next to the script
    /// it describes — the same attacker who can edit the script can
    /// usually edit the manifest sitting beside it. Only an explicit CLI
    /// flag, decided by whoever invokes `gx` rather than whoever wrote the
    /// script, may grant them; a manifest may still narrow the process
    /// allowlist once the CLI has granted `process`, exactly as before.
    pub fn apply_manifest(&mut self, manifest: &serde_json::Value) {
        let names = |key: &str| -> Option<Vec<String>> {
            manifest["dependencies"][key].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        };
        if let Some(list) = names("js") {
            self.js_modules = Allowlist::only(list);
        }
        if let Some(list) = names("ts") {
            self.ts_modules = Allowlist::only(list);
        }
        if let Some(list) = names("py") {
            self.py_modules = Allowlist::only(list);
        }
        if let Some(list) = names("binary") {
            self.binary_executables = Allowlist::only(list);
        }
        if let Some(list) = names("go") {
            self.go_executables = Allowlist::only(list);
        }
        if let Some(list) = names("rust_bin") {
            self.rust_bin_executables = Allowlist::only(list);
        }
        if let Some(list) = names("process") {
            self.process_executables = Allowlist::only(list);
        }
        if let Some(list) = names("ai") {
            self.ai_providers = Allowlist::only(list);
        }

        if let Some(v) = manifest["capabilities"]["external_network"].as_bool() {
            self.external_network = v;
        }
        if let Some(v) = manifest["capabilities"]["http_server"].as_bool() {
            self.http_server = v;
        }
        if let Some(v) = manifest["capabilities"]["database"].as_bool() {
            self.database = v;
        }
        if let Some(arr) = manifest["capabilities"]["env_deny"].as_array() {
            let denied: HashSet<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            self.environment = EnvAccess::Denied(denied);
        }
    }

    /// The raw sandbox directory, when filesystem access is scoped — for
    /// subsystems that need to construct or filter paths relative to the
    /// base themselves (e.g. `glob`), not just check one path at a time via
    /// [`resolve_path`](Self::resolve_path).
    pub fn sandbox_dir(&self) -> Option<&Path> {
        match &self.filesystem {
            FilesystemAccess::Sandboxed(base) => Some(base.as_path()),
            FilesystemAccess::Unrestricted => None,
        }
    }

    fn allowlist_for(&self, resource: Resource) -> Option<&Allowlist> {
        match resource {
            Resource::Process => Some(&self.process_executables),
            Resource::AiProviders => Some(&self.ai_providers),
            Resource::JsBridge => Some(&self.js_modules),
            Resource::TsBridge => Some(&self.ts_modules),
            Resource::PyBridge => Some(&self.py_modules),
            Resource::BinaryBridge => Some(&self.binary_executables),
            Resource::GoBridge => Some(&self.go_executables),
            Resource::RustBinBridge => Some(&self.rust_bin_executables),
            _ => None,
        }
    }

    /// The single entry point every dangerous runtime operation is
    /// authorized through. `name` scopes the check to a specific resource
    /// instance (an executable name, a bridge module, an AI provider) for
    /// the resources that have an allowlist; pass `None` for resources with
    /// no such scoping (shell, http_server, database, the network kinds).
    ///
    /// Subsystems call this and act on the result — they never decide for
    /// themselves whether something is authorized.
    pub fn authorize(&self, resource: Resource, name: Option<&str>) -> Result<(), Denial> {
        if self.denied.contains(&resource) {
            return Err(Denial::OperatorDenied { resource });
        }

        // Environment has denylist (not allowlist) semantics and needs the
        // specific variable name in its own error, so it doesn't fit the
        // generic bool-grant-then-allowlist shape below — handled here so
        // a denial always says *which* variable, not just "environment is
        // restricted".
        if resource == Resource::Environment {
            if let (EnvAccess::Denied(denied), Some(n)) = (&self.environment, name) {
                if denied.contains(n) {
                    return Err(Denial::EnvVarDenied {
                        name: n.to_string(),
                    });
                }
            }
            return Ok(());
        }

        let granted = match resource {
            Resource::Shell => self.shell,
            Resource::Process => self.process,
            Resource::Filesystem => true, // scope enforced by `resolve_path`, not a bool grant
            Resource::InternalNetwork => self.internal_network,
            Resource::ExternalNetwork => self.external_network,
            Resource::HttpServer => self.http_server,
            Resource::Database => self.database,
            Resource::Environment => unreachable!("handled above"),
            Resource::AiProviders
            | Resource::JsBridge
            | Resource::TsBridge
            | Resource::PyBridge
            | Resource::BinaryBridge
            | Resource::GoBridge
            | Resource::RustBinBridge => true, // gate is the allowlist below, not a separate bool
        };

        if !granted {
            return Err(Denial::NotGranted {
                resource,
                hint: not_granted_hint(resource),
            });
        }

        if let (Some(allowlist), Some(n)) = (self.allowlist_for(resource), name) {
            if !allowlist.permits(n) {
                return Err(Denial::NotInAllowlist {
                    resource,
                    name: n.to_string(),
                });
            }
        }

        Ok(())
    }

    /// Resolve `path_str` against the filesystem access scope, the same
    /// semantics `safe_path` had before the Capability Runtime existed:
    /// `Unrestricted` returns the path as-is; `Sandboxed(base)` requires the
    /// resolved absolute path to stay within `base`.
    pub fn resolve_path(&self, path_str: &str) -> Result<PathBuf, Denial> {
        self.authorize(Resource::Filesystem, None)?;
        let FilesystemAccess::Sandboxed(base) = &self.filesystem else {
            return Ok(PathBuf::from(path_str));
        };
        let raw = Path::new(path_str);
        let resolved = if raw.is_absolute() {
            normalize_path_no_symlink(raw)
        } else {
            normalize_path_no_symlink(&base.join(raw))
        };
        if !resolved.starts_with(base) {
            return Err(Denial::OutsideSandbox {
                path: resolved,
                sandbox: base.clone(),
            });
        }
        Ok(resolved)
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::new()
    }
}

fn not_granted_hint(resource: Resource) -> &'static str {
    match resource {
        Resource::Shell => "Run with --allow-shell to enable OS command execution.",
        Resource::Process => {
            "Run with --allow-process to enable process_run/process_spawn."
        }
        Resource::InternalNetwork => {
            "Use --allow-internal-http to allow requests to private/loopback addresses."
        }
        Resource::ExternalNetwork => {
            "Restricted via gx.json capabilities.external_network — remove that restriction to allow outbound network access."
        }
        Resource::HttpServer => {
            "Restricted via gx.json capabilities.http_server — remove that restriction to allow serve on port ..."
        }
        Resource::Database => {
            "Restricted via gx.json capabilities.database — remove that restriction to allow SQLite access."
        }
        _ => "This resource is restricted by gx.json.",
    }
}

/// Resolve `..`/`.` without touching the filesystem (no symlink resolution).
/// `pub(crate)` so other confinement checks (e.g. a package's declared
/// `entry` file staying inside its own package directory) can reuse the
/// same logic instead of re-deriving it.
pub(crate) fn normalize_path_no_symlink(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.last(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_pre_capability_runtime_behavior() {
        let caps = Capabilities::new();
        assert!(caps.authorize(Resource::Shell, None).is_err());
        assert!(caps.authorize(Resource::Process, None).is_err());
        assert!(caps.authorize(Resource::InternalNetwork, None).is_err());
        assert!(caps.authorize(Resource::ExternalNetwork, None).is_ok());
        assert!(caps.authorize(Resource::HttpServer, None).is_ok());
        assert!(caps.authorize(Resource::Database, None).is_ok());
        assert!(caps
            .authorize(Resource::Environment, Some("ANYTHING"))
            .is_ok());
        assert!(caps
            .authorize(Resource::AiProviders, Some("openai"))
            .is_ok());
        assert!(caps.authorize(Resource::JsBridge, Some("axios")).is_ok());
        assert!(caps.authorize(Resource::TsBridge, Some("anything")).is_ok());
        assert!(caps
            .authorize(Resource::BinaryBridge, Some("/any/path"))
            .is_ok());
    }

    #[test]
    fn allowlist_restricts_only_when_declared() {
        let mut caps = Capabilities::new();
        caps.process = true;
        assert!(caps.authorize(Resource::Process, Some("git")).is_ok());
        caps.process_executables = Allowlist::only(["git".to_string()]);
        assert!(caps.authorize(Resource::Process, Some("git")).is_ok());
        assert!(caps.authorize(Resource::Process, Some("curl")).is_err());
    }

    #[test]
    fn operator_deny_wins_over_every_other_grant() {
        let mut caps = Capabilities::new();
        caps.external_network = true; // explicitly granted
        caps.deny(Resource::ExternalNetwork);
        let err = caps.authorize(Resource::ExternalNetwork, None).unwrap_err();
        assert!(matches!(err, Denial::OperatorDenied { .. }));
    }

    #[test]
    fn operator_deny_cannot_be_reversed_by_reconfiguring_the_grant() {
        let mut caps = Capabilities::new();
        caps.deny(Resource::Shell);
        caps.shell = true; // even an explicit grant afterward doesn't help
        assert!(caps.authorize(Resource::Shell, None).is_err());
    }

    #[test]
    fn env_denylist_blocks_only_named_vars() {
        let mut caps = Capabilities::new();
        caps.environment = EnvAccess::Denied(["SECRET".to_string()].into_iter().collect());
        assert!(caps
            .authorize(Resource::Environment, Some("SECRET"))
            .is_err());
        assert!(caps.authorize(Resource::Environment, Some("PATH")).is_ok());
        // No name given means there's nothing to check against the
        // denylist — every current call site always passes a name, so this
        // only matters if a future caller doesn't.
        assert!(caps.authorize(Resource::Environment, None).is_ok());
    }

    #[test]
    fn resolve_path_unrestricted_passes_through() {
        let caps = Capabilities::new();
        assert_eq!(
            caps.resolve_path("relative/file.txt").unwrap(),
            PathBuf::from("relative/file.txt")
        );
    }

    #[test]
    fn resolve_path_sandboxed_rejects_escape() {
        let mut caps = Capabilities::new();
        caps.filesystem = FilesystemAccess::Sandboxed(PathBuf::from("/tmp/sandbox"));
        assert!(caps.resolve_path("inside.txt").is_ok());
        assert!(caps.resolve_path("../../etc/passwd").is_err());
        assert!(caps.resolve_path("/etc/passwd").is_err());
    }

    #[test]
    fn deny_filesystem_blocks_all_file_access_even_when_unrestricted() {
        // Regression test: resolve_path used to only consult
        // `self.filesystem`, never `self.denied` — so `--deny filesystem`
        // parsed and stored successfully but had no actual effect. An
        // operator override that silently does nothing is worse than one
        // that errors, since it looks like it's enforced.
        let mut caps = Capabilities::new();
        assert!(caps.resolve_path("anything.txt").is_ok());
        caps.deny(Resource::Filesystem);
        let err = caps.resolve_path("anything.txt").unwrap_err();
        assert!(matches!(err, Denial::OperatorDenied { .. }));
    }

    #[test]
    fn resource_name_roundtrips_through_parse() {
        let all = [
            Resource::Shell,
            Resource::Process,
            Resource::Filesystem,
            Resource::InternalNetwork,
            Resource::ExternalNetwork,
            Resource::HttpServer,
            Resource::Database,
            Resource::Environment,
            Resource::AiProviders,
            Resource::JsBridge,
            Resource::TsBridge,
            Resource::PyBridge,
            Resource::BinaryBridge,
            Resource::GoBridge,
            Resource::RustBinBridge,
        ];
        for r in all {
            assert_eq!(Resource::parse(r.name()), Some(r));
        }
        assert_eq!(Resource::parse("not-a-real-resource"), None);
    }

    #[test]
    fn apply_manifest_restricts_only_declared_dependency_lists() {
        let mut caps = Capabilities::new();
        // Untouched namespaces stay open.
        assert!(caps.authorize(Resource::PyBridge, Some("anything")).is_ok());
        let manifest: serde_json::Value = serde_json::json!({
            "dependencies": { "js": ["axios"], "binary": ["./svc"] }
        });
        caps.apply_manifest(&manifest);
        assert!(caps.authorize(Resource::JsBridge, Some("axios")).is_ok());
        assert!(caps
            .authorize(Resource::JsBridge, Some("left-pad"))
            .is_err());
        assert!(caps
            .authorize(Resource::BinaryBridge, Some("./svc"))
            .is_ok());
        assert!(caps
            .authorize(Resource::BinaryBridge, Some("/bin/sh"))
            .is_err());
        // py wasn't declared — still open.
        assert!(caps.authorize(Resource::PyBridge, Some("anything")).is_ok());
    }

    #[test]
    fn apply_manifest_cannot_grant_shell_process_or_internal_network() {
        let mut caps = Capabilities::new();
        let manifest: serde_json::Value = serde_json::json!({
            "capabilities": { "shell": true, "process": true, "internal_network": true }
        });
        caps.apply_manifest(&manifest);
        assert!(caps.authorize(Resource::Shell, None).is_err());
        assert!(caps.authorize(Resource::Process, None).is_err());
        assert!(caps.authorize(Resource::InternalNetwork, None).is_err());
    }

    #[test]
    fn apply_manifest_can_restrict_previously_open_resources() {
        let mut caps = Capabilities::new();
        assert!(caps.authorize(Resource::HttpServer, None).is_ok());
        let manifest: serde_json::Value = serde_json::json!({
            "capabilities": {
                "http_server": false,
                "database": false,
                "external_network": false,
                "env_deny": ["SECRET_TOKEN"]
            }
        });
        caps.apply_manifest(&manifest);
        assert!(caps.authorize(Resource::HttpServer, None).is_err());
        assert!(caps.authorize(Resource::Database, None).is_err());
        assert!(caps.authorize(Resource::ExternalNetwork, None).is_err());
        assert!(caps
            .authorize(Resource::Environment, Some("SECRET_TOKEN"))
            .is_err());
        assert!(caps.authorize(Resource::Environment, Some("PATH")).is_ok());
    }
}
