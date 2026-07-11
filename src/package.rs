//! The Module & Package Runtime: `gx.json` package metadata, semantic
//! versioning, `gx.lock` (dependency locking for reproducible builds),
//! package integrity verification, and the local package cache.
//!
//! ## What a GX package is
//!
//! A directory containing a `gx.json` (top-level `name`/`version`/
//! `description`/`entry` — the same fields `gx init` has scaffolded since
//! before this milestone, previously entirely inert) plus `.gx` source
//! files. `entry` (default `"main.gx"`) is the file another project's
//! `import "that-package-name"` resolves to.
//!
//! ## Where packages come from
//!
//! Three source kinds, declared per-dependency in `gx.json`'s
//! `dependencies.gx`:
//!
//! - A semver range string (`"^1.2.0"`) — resolved against whatever
//!   versions are already in the local cache (`~/.gx/packages/<name>/`).
//!   Nothing is ever fetched implicitly; `gx install` is what populates
//!   the cache.
//! - `{ "git": "<url>", "rev": "<ref>" }` — cloned into the cache by `gx
//!   install` (shells out to `git`, the same "call the real tool" pattern
//!   the existing `js`/`py` bridge installers already use).
//! - `{ "path": "<relative path>" }` — used directly, never cached; for
//!   local, in-monorepo development.
//!
//! There is deliberately no hosted registry here — GX has no server
//! infrastructure to publish one, and inventing a fake one would be
//! exactly the kind of feature built "because other languages have it"
//! this milestone was told not to build. Git-based distribution (clone a
//! tagged repository) is a real, complete, *offline-capable-after-first-
//! fetch* package source that needs no new infrastructure at all — the
//! same bootstrap path early Cargo used before crates.io existed.
//!
//! ## Reproducibility and integrity
//!
//! `gx.lock` pins the exact resolved version and a SHA-256 content hash
//! per dependency. Every subsequent resolution (`gx run`, `gx build`) that
//! finds a lockfile re-verifies the cached package's hash against it
//! before using it — a tampered or corrupted local cache entry is
//! rejected, not silently used (path dependencies are the one exception:
//! see `Interpreter::resolve_package_import_impl`'s comment on why they're
//! never hash-checked).
//!
//! ## Capability scope (deliberate boundary)
//!
//! Imported/dependency code — whether a plain file import, a namespaced
//! module import, or a package import resolved through this module — runs
//! with exactly the same [`crate::capability::Capabilities`] grant as the
//! script that imported it. There is no per-package capability sandboxing:
//! a dependency's functions execute in the same `Interpreter`, sharing its
//! one capability set, the same way an ordinary function call does. This
//! is a deliberate scope boundary for this milestone, not an oversight —
//! isolating each dependency in its own capability sandbox (so a leftpad-
//! style utility couldn't open sockets just because the top-level program
//! is allowed to) is a real, valuable, but substantially larger feature:
//! it would need per-module capability declarations in each dependency's
//! own `gx.json`, a way to intersect/attenuate a parent's grant when
//! calling into a child, and enforcement at every call boundary rather
//! than once at process start. Worth doing, but as its own milestone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const LOCKFILE_VERSION: u64 = 1;
pub const LOCKFILE_NAME: &str = "gx.lock";
pub const MANIFEST_NAME: &str = "gx.json";
const DEFAULT_ENTRY: &str = "main.gx";

// ── Local cache location ──────────────────────────────────────────────────────

fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
}

/// Root of the local package cache. `GX_PACKAGE_CACHE_DIR` overrides it
/// entirely (used by this module's own tests, and available for CI/sandboxed
/// environments that want every bit of GX's local state redirected) —
/// mirrors the existing `GX_STATE_DIR` override for persistent memory.
pub fn cache_root() -> PathBuf {
    if let Ok(dir) = std::env::var("GX_PACKAGE_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    let home = home_dir().unwrap_or_else(|| ".".to_string());
    PathBuf::from(home).join(".gx").join("packages")
}

/// A name/version turned into a filesystem-safe path segment — defense
/// against path traversal via a malicious or malformed `gx.json` (a
/// dependency literally named `"../../../etc"`, or a git rev containing
/// `../`) landing outside the cache root.
fn sanitize_path_segment(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '@' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // A cleaned segment that's entirely dots (".", "..", "...") could still
    // navigate the filesystem even with disallowed characters stripped —
    // collapse any all-dot result to a literal, inert string instead.
    if cleaned.chars().all(|c| c == '.') || cleaned.is_empty() {
        "_".to_string()
    } else {
        cleaned
    }
}

pub fn cache_dir_for(cache_root: &Path, name: &str, version: &str) -> PathBuf {
    cache_root
        .join(sanitize_path_segment(name))
        .join(sanitize_path_segment(version))
}

// ── Package metadata (gx.json's name/version/description/entry) ─────────────

#[derive(Debug, Clone, Default)]
pub struct PackageMetadata {
    pub name: Option<String>,
    pub version: Option<semver::Version>,
    pub description: Option<String>,
    pub entry: String,
}

impl PackageMetadata {
    pub fn from_manifest(manifest: &serde_json::Value) -> Result<Self, String> {
        let name = manifest["name"].as_str().map(String::from);
        let version = match manifest["version"].as_str() {
            Some(v) => Some(
                semver::Version::parse(v)
                    .map_err(|e| format!("gx.json: invalid 'version' \"{}\": {}", v, e))?,
            ),
            None => None,
        };
        let description = manifest["description"].as_str().map(String::from);
        let entry = manifest["entry"]
            .as_str()
            .unwrap_or(DEFAULT_ENTRY)
            .to_string();
        Ok(PackageMetadata {
            name,
            version,
            description,
            entry,
        })
    }
}

// ── Dependency specs (gx.json's dependencies.gx) ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DependencySource {
    /// A semver range string, resolved against the local cache.
    Registry(String),
    Git {
        url: String,
        rev: Option<String>,
    },
    Path(String),
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub source: DependencySource,
}

/// Parses `gx.json`'s `dependencies.gx`. A missing key, or one that isn't a
/// JSON object, is treated as "no GX package dependencies declared" rather
/// than an error — this deliberately also covers the legacy `"gx": []`
/// placeholder `gx init` scaffolded before this milestone (an empty array,
/// never read by anything), so every existing generated project keeps
/// working unchanged.
pub fn parse_gx_dependencies(manifest: &serde_json::Value) -> Result<Vec<Dependency>, String> {
    let Some(obj) = manifest["dependencies"]["gx"].as_object() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (name, spec) in obj {
        let source = match spec {
            serde_json::Value::String(range) => {
                // Validate eagerly so a typo'd range fails at parse time
                // with a clear message, not deep inside resolution.
                semver::VersionReq::parse(range).map_err(|e| {
                    format!(
                        "gx.json: dependency '{}' has an invalid version range \"{}\": {}",
                        name, range, e
                    )
                })?;
                DependencySource::Registry(range.clone())
            }
            serde_json::Value::Object(o) => {
                if let Some(git) = o.get("git").and_then(|v| v.as_str()) {
                    DependencySource::Git {
                        url: git.to_string(),
                        rev: o.get("rev").and_then(|v| v.as_str()).map(String::from),
                    }
                } else if let Some(path) = o.get("path").and_then(|v| v.as_str()) {
                    DependencySource::Path(path.to_string())
                } else {
                    return Err(format!(
                        "gx.json: dependency '{}' must have a 'git' or 'path' field",
                        name
                    ));
                }
            }
            other => {
                return Err(format!(
                    "gx.json: dependency '{}' must be a version range string or an object, got {}",
                    name, other
                ))
            }
        };
        out.push(Dependency {
            name: name.clone(),
            source,
        });
    }
    // Sorted by name for deterministic resolution order — dependency
    // resolution order can otherwise vary with HashMap/JSON-object
    // iteration order, which has no defined ordering.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Every cached version directory for `name` under `cache_root`, parsed as
/// semver and sorted ascending. Unparseable directory names (foreign
/// contents someone put in the cache by hand) are skipped rather than
/// failing the whole scan. Takes `cache_root` explicitly (rather than
/// calling `cache_root()` itself) so tests can point it at an isolated
/// temp directory without mutating the process-global `GX_PACKAGE_CACHE_DIR`
/// environment variable — safe under `cargo test`'s default parallelism.
pub fn cached_versions(cache_root: &Path, name: &str) -> Vec<semver::Version> {
    let dir = cache_root.join(sanitize_path_segment(name));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut versions: Vec<semver::Version> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|s| semver::Version::parse(&s).ok())
        .collect();
    versions.sort();
    versions
}

/// The highest cached version of `name` satisfying `range`. `None` means
/// nothing satisfying it is cached — the caller (`gx install`'s consumer,
/// or the import resolver) is responsible for turning that into a clear
/// "run `gx install`" style error rather than silently doing nothing.
pub fn resolve_registry_version(
    cache_root: &Path,
    name: &str,
    range: &str,
) -> Result<Option<semver::Version>, String> {
    let req = semver::VersionReq::parse(range)
        .map_err(|e| format!("invalid version range \"{}\": {}", range, e))?;
    Ok(cached_versions(cache_root, name)
        .into_iter()
        .rev()
        .find(|v| req.matches(v)))
}

// ── Integrity hashing ─────────────────────────────────────────────────────────

/// A stable SHA-256 over every file in `root`'s tree (sorted by relative
/// path, so directory-walk order never affects the result), skipping
/// hidden entries (`.git`, `.DS_Store`, ...) and `gx.lock` itself (a
/// package doesn't hash its own lock — that would be circular, and a
/// dependency's own `gx.lock`, if it has one, is irrelevant to *this*
/// project's lock). This is what "package integrity verification" checks
/// against: a cached package whose content has changed since it was
/// locked — corruption, or local tampering — is detected, not silently
/// trusted.
pub fn hash_package_tree(root: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for rel in &files {
        let content = std::fs::read(root.join(rel))
            .map_err(|e| format!("cannot read '{}': {}", rel.display(), e))?;
        // Length-prefix the path so "a" + "b/c" can never hash the same as
        // "a/b" + "c" — a real (if obscure) collision risk for a naive
        // path||content concatenation.
        let rel_str = rel.to_string_lossy();
        hasher.update((rel_str.len() as u64).to_le_bytes());
        hasher.update(rel_str.as_bytes());
        hasher.update((content.len() as u64).to_le_bytes());
        hasher.update(&content);
    }
    let digest = hasher.finalize();
    Ok(format!("sha256-{}", hex_encode(&digest)))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot read '{}': {}", dir.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        if name_str.starts_with('.') || name_str == LOCKFILE_NAME {
            continue;
        }
        // `DirEntry::file_type()` reports the entry itself, never
        // following a symlink (unlike `path.is_dir()`/`fs::read`, which
        // both follow it). A symlink is rejected outright rather than
        // resolved-and-range-checked: an untrusted git/registry
        // dependency could otherwise plant one pointing outside the
        // package (e.g. a `main.gx` symlinked to `~/.ssh/id_rsa`) that
        // would later be read as this package's own source the moment
        // something imports it — a local file disclosure primitive. Real
        // packages essentially never need an internal symlink, so simply
        // refusing them (as several other ecosystems' publish tooling
        // already does) is both the simplest and the safest rule.
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat '{}': {}", path.display(), e))?;
        if file_type.is_symlink() {
            return Err(format!(
                "'{}' contains a symlink ('{}'), which is not allowed in a package's hashed \
                 tree — it could point outside the package and be read as part of it",
                root.display(),
                path.display()
            ));
        }
        if file_type.is_dir() {
            collect_files(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ── gx.lock ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LockedPackage {
    pub version: String,
    pub resolved: String,
    pub integrity: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LockFile {
    pub version: u64,
    pub packages: BTreeMap<String, LockedPackage>,
}

impl LockFile {
    pub fn new() -> Self {
        LockFile {
            version: LOCKFILE_VERSION,
            packages: BTreeMap::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;
        let lock: LockFile = serde_json::from_str(&content)
            .map_err(|e| format!("'{}' is not a valid gx.lock: {}", path.display(), e))?;
        if lock.version != LOCKFILE_VERSION {
            return Err(format!(
                "'{}' was written by an incompatible gx.lock format (version {}, this GX supports version {}) — delete it and run `gx install` again",
                path.display(),
                lock.version,
                LOCKFILE_VERSION
            ));
        }
        Ok(Some(lock))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        // Pretty-printed and key-sorted (BTreeMap already sorts) so
        // `gx.lock` diffs cleanly in version control — a lockfile that
        // reordered itself on every regeneration would defeat much of the
        // point of committing it.
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize gx.lock: {}", e))?;
        std::fs::write(path, content + "\n")
            .map_err(|e| format!("cannot write '{}': {}", path.display(), e))
    }
}

impl Default for LockFile {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "gx_package_test_{}_{}_{}",
            label,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sanitize_path_segment_neutralizes_traversal_attempts() {
        assert_eq!(sanitize_path_segment("normal-name"), "normal-name");
        // `/` becomes `_` (no separators survive, so the whole thing is one
        // inert path segment); `.` itself is an allowed character (dots are
        // common and harmless in real package/version names), so isolated
        // ".." sequences remain — but never adjacent to a surviving
        // separator, so they can never actually mean "parent directory".
        assert_eq!(
            sanitize_path_segment("../../etc/passwd"),
            ".._.._etc_passwd"
        );
        assert!(!sanitize_path_segment("../../etc/passwd").contains('/'));
        assert_eq!(sanitize_path_segment(".."), "_");
        assert_eq!(sanitize_path_segment("."), "_");
        assert_eq!(sanitize_path_segment("...."), "_");
        assert_eq!(sanitize_path_segment(""), "_");
        assert_eq!(sanitize_path_segment("@scope/pkg"), "@scope_pkg");
    }

    #[test]
    fn cache_dir_for_never_escapes_the_cache_root_even_with_hostile_input() {
        let root = PathBuf::from("/tmp/gx_test_cache_root");
        let dir = cache_dir_for(&root, "../../../etc", "../../passwd");
        assert!(
            dir.starts_with(&root),
            "cache_dir_for('../../../etc', '../../passwd') = {} must stay under {}",
            dir.display(),
            root.display()
        );
    }

    #[test]
    fn parse_gx_dependencies_handles_every_source_kind() {
        let manifest = serde_json::json!({
            "dependencies": {
                "gx": {
                    "range-lib": "^1.2.0",
                    "git-lib": { "git": "https://example.com/repo", "rev": "abc123" },
                    "path-lib": { "path": "../local" }
                }
            }
        });
        let deps = parse_gx_dependencies(&manifest).unwrap();
        assert_eq!(deps.len(), 3);
        let by_name: std::collections::HashMap<_, _> = deps
            .into_iter()
            .map(|d| (d.name.clone(), d.source))
            .collect();
        assert_eq!(
            by_name["range-lib"],
            DependencySource::Registry("^1.2.0".to_string())
        );
        assert_eq!(
            by_name["git-lib"],
            DependencySource::Git {
                url: "https://example.com/repo".to_string(),
                rev: Some("abc123".to_string())
            }
        );
        assert_eq!(
            by_name["path-lib"],
            DependencySource::Path("../local".to_string())
        );
    }

    #[test]
    fn parse_gx_dependencies_treats_the_legacy_empty_array_as_no_dependencies() {
        // gx init has scaffolded `"gx": []` since before this milestone —
        // nothing ever read it. Must not error now that something does.
        let manifest = serde_json::json!({ "dependencies": { "gx": [] } });
        assert_eq!(parse_gx_dependencies(&manifest).unwrap().len(), 0);

        let no_deps_at_all = serde_json::json!({});
        assert_eq!(parse_gx_dependencies(&no_deps_at_all).unwrap().len(), 0);
    }

    #[test]
    fn parse_gx_dependencies_rejects_an_invalid_version_range_with_a_clear_message() {
        let manifest = serde_json::json!({
            "dependencies": { "gx": { "bad-lib": "not-a-valid-range!!" } }
        });
        let err = parse_gx_dependencies(&manifest).unwrap_err();
        assert!(err.contains("bad-lib"));
        assert!(err.contains("not-a-valid-range!!"));
    }

    #[test]
    fn parse_gx_dependencies_rejects_an_object_with_neither_git_nor_path() {
        let manifest = serde_json::json!({
            "dependencies": { "gx": { "mystery-lib": { "unknown_field": "x" } } }
        });
        assert!(parse_gx_dependencies(&manifest).is_err());
    }

    #[test]
    fn package_metadata_parses_the_existing_gx_init_fields() {
        let manifest = serde_json::json!({
            "name": "my-lib",
            "version": "1.2.3",
            "description": "A test library",
            "entry": "lib.gx"
        });
        let meta = PackageMetadata::from_manifest(&manifest).unwrap();
        assert_eq!(meta.name, Some("my-lib".to_string()));
        assert_eq!(meta.version, Some(semver::Version::new(1, 2, 3)));
        assert_eq!(meta.description, Some("A test library".to_string()));
        assert_eq!(meta.entry, "lib.gx");
    }

    #[test]
    fn package_metadata_defaults_entry_to_main_gx() {
        let manifest = serde_json::json!({});
        let meta = PackageMetadata::from_manifest(&manifest).unwrap();
        assert_eq!(meta.entry, "main.gx");
        assert_eq!(meta.name, None);
        assert_eq!(meta.version, None);
    }

    #[test]
    fn package_metadata_rejects_an_invalid_semver_version() {
        let manifest = serde_json::json!({ "version": "not-a-version" });
        assert!(PackageMetadata::from_manifest(&manifest).is_err());
    }

    #[test]
    fn resolve_registry_version_picks_the_highest_match_and_ignores_the_rest() {
        let dir = temp_dir("resolve-version");
        for v in ["1.0.0", "1.5.0", "2.0.0", "1.9.9"] {
            std::fs::create_dir_all(dir.join("my-lib").join(v)).unwrap();
        }
        let resolved = resolve_registry_version(&dir, "my-lib", "^1.0.0").unwrap();
        assert_eq!(resolved, Some(semver::Version::new(1, 9, 9)));
        let none_match = resolve_registry_version(&dir, "my-lib", "^3.0.0").unwrap();
        assert_eq!(none_match, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hash_package_tree_is_stable_and_detects_any_content_change() {
        let dir = temp_dir("hash-stable");
        std::fs::write(dir.join("main.gx"), "function f() { return 1 }").unwrap();
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("lib").join("util.gx"), "function g() { return 2 }").unwrap();
        // Noise that must be ignored: hidden files/dirs and gx.lock itself.
        std::fs::write(dir.join(".DS_Store"), "junk").unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git").join("HEAD"), "ref: refs/heads/main").unwrap();
        std::fs::write(dir.join(LOCKFILE_NAME), "{}").unwrap();

        let hash1 = hash_package_tree(&dir).unwrap();
        let hash2 = hash_package_tree(&dir).unwrap();
        assert_eq!(
            hash1, hash2,
            "hashing the same unchanged tree twice must be stable"
        );
        assert!(hash1.starts_with("sha256-"));

        std::fs::write(dir.join("main.gx"), "function f() { return 999 }").unwrap();
        let hash3 = hash_package_tree(&dir).unwrap();
        assert_ne!(
            hash1, hash3,
            "changing a file's content must change the hash"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hash_package_tree_is_independent_of_directory_walk_order() {
        // Two logically-identical trees, populated in a different order,
        // must hash identically — proves the sort-before-hash actually
        // works, not just "usually happens to work" on one filesystem's
        // directory-entry ordering.
        let dir_a = temp_dir("hash-order-a");
        std::fs::write(dir_a.join("z.gx"), "1").unwrap();
        std::fs::write(dir_a.join("a.gx"), "2").unwrap();

        let dir_b = temp_dir("hash-order-b");
        std::fs::write(dir_b.join("a.gx"), "2").unwrap();
        std::fs::write(dir_b.join("z.gx"), "1").unwrap();

        assert_eq!(
            hash_package_tree(&dir_a).unwrap(),
            hash_package_tree(&dir_b).unwrap()
        );
        std::fs::remove_dir_all(&dir_a).ok();
        std::fs::remove_dir_all(&dir_b).ok();
    }

    #[test]
    #[cfg(unix)]
    fn hash_package_tree_rejects_a_symlink_escaping_the_package() {
        // Regression test for a real local-file-disclosure primitive: an
        // untrusted git/registry dependency could otherwise plant a
        // symlink (e.g. `main.gx` -> `~/.ssh/id_rsa`) that would later be
        // read as this package's own source the moment something imports
        // it. `hash_package_tree` must refuse the whole tree rather than
        // silently following the link.
        let outside = temp_dir("symlink-escape-target");
        std::fs::write(outside.join("secret.txt"), "top secret").unwrap();

        let dir = temp_dir("symlink-escape-package");
        std::fs::write(dir.join("main.gx"), "function f() { return 1 }").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), dir.join("evil.gx")).unwrap();

        let err = hash_package_tree(&dir).unwrap_err();
        assert!(
            err.contains("symlink"),
            "expected a symlink rejection, got: {}",
            err
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn lockfile_round_trips_through_json_and_rejects_a_future_version() {
        let dir = temp_dir("lockfile");
        let path = dir.join(LOCKFILE_NAME);

        let mut lock = LockFile::new();
        lock.packages.insert(
            "my-lib".to_string(),
            LockedPackage {
                version: "1.2.3".to_string(),
                resolved: "git+https://example.com/repo#abc123".to_string(),
                integrity: "sha256-deadbeef".to_string(),
            },
        );
        lock.save(&path).unwrap();

        let loaded = LockFile::load(&path).unwrap().unwrap();
        assert_eq!(loaded.version, LOCKFILE_VERSION);
        assert_eq!(loaded.packages["my-lib"].version, "1.2.3");
        assert_eq!(loaded.packages["my-lib"].integrity, "sha256-deadbeef");

        // A lockfile from an incompatible future format must be rejected,
        // not silently misread.
        std::fs::write(&path, r#"{"version": 999999, "packages": {}}"#).unwrap();
        let err = LockFile::load(&path).unwrap_err();
        assert!(err.contains("incompatible"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lockfile_load_returns_none_when_the_file_does_not_exist() {
        let dir = temp_dir("lockfile-missing");
        let result = LockFile::load(&dir.join(LOCKFILE_NAME)).unwrap();
        assert!(result.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
