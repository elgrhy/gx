//! GX Toolchain — gx init, gx build, gx install, gx fmt, gx make, gx test

use std::fs;
use std::path::Path;
use std::process::Command;

// ── gx init ───────────────────────────────────────────────────────────────────

pub fn init(name: &str) -> Result<(), String> {
    let dir = Path::new(name);
    if dir.exists() {
        return Err(format!("Directory '{}' already exists", name));
    }

    fs::create_dir_all(dir.join("agents"))
        .and(fs::create_dir_all(dir.join("tests")))
        .map_err(|e| format!("Failed to create directories: {}", e))?;

    // gx.json — "capabilities" is scaffolded with values that match the
    // runtime's own safe-by-default grants exactly (see
    // Capabilities::new() in src/capability.rs), so a fresh project's
    // behavior is completely unchanged from before this block existed.
    // It's here so the ability to *restrict* — e.g. set "external_network"
    // to false for a script that should only ever talk to localhost, or
    // add names to "env_deny" to keep specific secrets out of `env()`
    // reach — is discoverable in every new project without reading
    // Rust source or the language reference first. shell/process/
    // internal_network are deliberately not listed here: those are
    // CLI-only grants (`--allow-shell`/`--allow-process`/
    // `--allow-internal-http`) precisely so the entity invoking `gx`,
    // not the script's own manifest, has the final say over them.
    let manifest = format!(
        r#"{{
  "name": "{}",
  "version": "0.1.0",
  "description": "A GX agent project",
  "entry": "main.gx",
  "dependencies": {{
    "js": [],
    "py": [],
    "gx": []
  }},
  "capabilities": {{
    "external_network": true,
    "http_server": true,
    "database": true,
    "env_deny": []
  }}
}}
"#,
        name
    );
    fs::write(dir.join("gx.json"), &manifest)
        .map_err(|e| format!("Failed to write gx.json: {}", e))?;

    // main.gx
    let main_gx = format!(
        r#"// {} — GX project entry point
// Run with: gx run main.gx

agent "{}" {{
  remember {{
    name = "{}"
    started_at = 0
  }}

  when started {{
    memory.started_at = get_timestamp()
    say "Hello from {{memory.name}}! Agent started."
  }}

  brain {{
    plan {{
      plan = {{ action: "idle" }}
    }}

    execute {{
      if plan.action == "idle" {{
        log("{{memory.name}} is ready.")
      }}
    }}

    remember {{
      memory.last_run = get_timestamp()
    }}

    communicate {{
      emit "agent_ready" {{ name: memory.name }}
    }}
  }}
}}
"#,
        name, name, name
    );
    fs::write(dir.join("main.gx"), &main_gx)
        .map_err(|e| format!("Failed to write main.gx: {}", e))?;

    // tests/test_basic.gx
    let test_gx = format!(
        r#"// Basic test for {}
helper "test_basic" {{
  brain {{
    plan {{ plan = {{ action: "test" }} }}
    execute {{
      if plan.action == "test" {{
        result = 1 + 1
        if result == 2 {{
          log("PASS: arithmetic works")
        }} else {{
          log("FAIL: arithmetic broken")
        }}
      }}
    }}
    remember {{ }}
    communicate {{ }}
  }}
}}
"#,
        name
    );
    fs::write(dir.join("tests").join("test_basic.gx"), &test_gx)
        .map_err(|e| format!("Failed to write test: {}", e))?;

    // .gitignore
    fs::write(dir.join(".gitignore"), "dist/\n.gx_cache/\n")
        .map_err(|e| format!("Failed to write .gitignore: {}", e))?;

    println!("Created GX project '{}'", name);
    println!();
    println!("  cd {}", name);
    println!("  gx run main.gx");
    println!();
    println!("Files created:");
    println!("  {}/main.gx        — entry point", name);
    println!("  {}/gx.json        — project manifest", name);
    println!("  {}/agents/        — put your agents here", name);
    println!("  {}/tests/         — test files", name);

    Ok(())
}

/// Truncate `s` to at most `max` bytes without panicking when `max` lands
/// inside a multi-byte UTF-8 character — `&s[..max]` alone panics in that
/// case, and `s` here is raw AI-response text (`gx make`), which routinely
/// contains multi-byte characters (curly quotes, em dashes) near arbitrary
/// byte offsets.
fn truncate_at_char_boundary(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ── gx build ─────────────────────────────────────────────────────────────────

/// Picks a heredoc delimiter for embedding `source` into the launcher shell
/// script, drawing from `candidates` and skipping any that collide with one
/// of `source`'s own lines (see `build`'s heredoc-injection comment for why
/// this check exists). `candidates` is a parameter — rather than generating
/// UUIDs inline — so the collision-skipping behavior itself is deterministic
/// and unit-testable independent of UUID randomness.
fn pick_heredoc_delimiter(source: &str, mut candidates: impl Iterator<Item = String>) -> String {
    loop {
        let candidate = candidates
            .next()
            .expect("candidate stream must be infinite");
        if !source.lines().any(|line| line == candidate) {
            return candidate;
        }
    }
}

/// `--allow-shell`/`--allow-process`/`--allow-internal-http`/`--deny` are
/// accepted at build time, not run time, because a distributed binary's
/// end user generally has no way to know which flags the program actually
/// needs — the developer who built it does. Whatever's passed here is
/// baked into the launcher's own invocation of `gx run`. This still can't
/// grant more than a `gx run` invocation could: it's the exact same flags,
/// just decided once at build time instead of by whoever runs the binary.
#[allow(clippy::too_many_arguments)]
pub fn build(
    file: &str,
    output: Option<&str>,
    allow_shell: bool,
    allow_process: bool,
    allow_internal_http: bool,
    deny: Vec<crate::capability::Resource>,
) -> Result<(), String> {
    if !Path::new(file).exists() {
        return Err(format!("File not found: {}", file));
    }

    // Find the GX binary (ourselves)
    let gx_binary = std::env::current_exe().map_err(|e| format!("Cannot find gx binary: {}", e))?;

    let dist_dir = Path::new("dist");
    fs::create_dir_all(dist_dir).map_err(|e| format!("Cannot create dist/: {}", e))?;

    let stem = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    let out_name = output.unwrap_or(stem);
    let out_path = if cfg!(windows) {
        dist_dir.join(format!("{}.exe", out_name))
    } else {
        dist_dir.join(out_name)
    };

    // Strategy: create a shell script that bundles the gx binary + source
    // For now, create a self-contained launcher script
    let source = fs::read_to_string(file).map_err(|e| format!("Cannot read {}: {}", file, e))?;

    // A gx.json next to the source declares this program's dependency/
    // capability allowlists (see crate::capability). Copy it into dist/ so
    // the built launcher still finds it — without this, the manifest was
    // silently dropped the moment source got embedded into a standalone
    // launcher, and every dependencies.*/capabilities.* restriction the
    // developer declared would quietly stop applying to the built binary.
    if let Some(src_dir) = Path::new(file).parent() {
        let manifest_src = src_dir.join("gx.json");
        if manifest_src.exists() {
            fs::copy(&manifest_src, dist_dir.join("gx.json"))
                .map_err(|e| format!("Cannot copy gx.json into dist/: {}", e))?;
        }
    }

    let mut run_flags = String::new();
    if allow_shell {
        run_flags.push_str(" --allow-shell");
    }
    if allow_process {
        run_flags.push_str(" --allow-process");
    }
    if allow_internal_http {
        run_flags.push_str(" --allow-internal-http");
    }
    for resource in &deny {
        run_flags.push_str(&format!(" --deny {}", resource.name()));
    }

    // Embed source via a here-document piped to `gx run -` (stdin mode).
    // This eliminates the TOCTOU race and avoids leaking source in /tmp.
    // The GX runtime supports `gx run -` to read source from stdin. `cd`
    // into the launcher's own directory first so `gx run -`'s manifest/
    // sandbox resolution (which uses cwd for stdin mode) finds the gx.json
    // copied alongside it above, regardless of where the user invokes the
    // launcher from.
    //
    // The heredoc delimiter must not collide with any line the embedded
    // source contains — `sh` ends a heredoc the moment it sees a line that
    // *is* the delimiter, so a `.gx` file with a line equal to a fixed,
    // guessable delimiter (a comment or string literal containing it,
    // deliberately or not) would terminate the heredoc early and turn
    // everything after it into literal, unsandboxed shell script executed
    // by `/bin/sh` — a complete Capability Runtime bypass the moment the
    // built launcher runs. A UUID-derived delimiter, checked against every
    // line of the actual source before use, closes this: collision is
    // astronomically unlikely, and explicitly verified rather than assumed.
    let delimiter = pick_heredoc_delimiter(
        &source,
        std::iter::repeat_with(|| format!("__GX_SOURCE_{}__", uuid::Uuid::new_v4().simple())),
    );
    let launcher = format!(
        r#"#!/bin/sh
# Self-contained GX program: {}
# Generated by gx build

GX_BINARY="{}"
cd "$(dirname "$0")"

"$GX_BINARY" run -{} <<'{}'
{}
{}
"#,
        file,
        gx_binary.display(),
        run_flags,
        delimiter,
        source,
        delimiter
    );

    fs::write(&out_path, &launcher).map_err(|e| format!("Cannot write output: {}", e))?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&out_path)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&out_path, perms)
            .map_err(|e| format!("Cannot set permissions: {}", e))?;
    }

    println!("Built: {}", out_path.display());
    println!("Run:   {}", out_path.display());

    Ok(())
}

// ── gx install ────────────────────────────────────────────────────────────────

/// `gx install` with no package argument: resolve every dependency
/// declared in `gx.json`'s `dependencies.gx` against the local package
/// cache (fetching git dependencies, verifying path dependencies exist),
/// and write `gx.lock` pinning the exact resolved version + integrity hash
/// of each — the "reproducible builds" half of this milestone. `gx install
/// js.X`/`py.X` (unchanged, see `install` below) remains the way to add a
/// *bridge* package; this is deliberately a separate entry point rather
/// than overloading the same one-string-argument CLI shape, since
/// resolving-everything-in-the-manifest and adding-one-new-thing are
/// different operations with different inputs.
///
/// Native-only: relies on `crate::package` (semver/lockfile/cache) and
/// shelling out to `git`, neither available under `wasm32`.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_all(offline: bool) -> Result<(), String> {
    let manifest_path = Path::new(crate::package::MANIFEST_NAME);
    if !manifest_path.exists() {
        println!("No gx.json found in the current directory — nothing to install.");
        return Ok(());
    }
    let manifest_dir = manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let content = fs::read_to_string(manifest_path)
        .map_err(|e| format!("Cannot read {}: {}", crate::package::MANIFEST_NAME, e))?;
    let manifest: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("{} is not valid JSON: {}", crate::package::MANIFEST_NAME, e))?;
    let deps = crate::package::parse_gx_dependencies(&manifest)?;

    if deps.is_empty() {
        println!("No GX package dependencies declared in gx.json's dependencies.gx — nothing to install.");
        return Ok(());
    }

    let cache_root = crate::package::cache_root();
    let mut lock = crate::package::LockFile::new();
    for dep in &deps {
        println!("Resolving {}...", dep.name);
        let (version, resolved, dir) =
            resolve_one_dependency(dep, manifest_dir, &cache_root, offline)?;
        let integrity = crate::package::hash_package_tree(&dir)
            .map_err(|e| format!("failed to hash '{}': {}", dep.name, e))?;
        println!("  -> {} {} ({})", dep.name, version, resolved);
        lock.packages.insert(
            dep.name.clone(),
            crate::package::LockedPackage {
                version,
                resolved,
                integrity,
            },
        );
    }
    lock.save(&manifest_dir.join(crate::package::LOCKFILE_NAME))?;
    println!(
        "Wrote {} ({} package{})",
        crate::package::LOCKFILE_NAME,
        deps.len(),
        if deps.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_one_dependency(
    dep: &crate::package::Dependency,
    manifest_dir: &Path,
    cache_root: &Path,
    offline: bool,
) -> Result<(String, String, std::path::PathBuf), String> {
    use crate::package::DependencySource;
    match &dep.source {
        DependencySource::Path(p) => {
            let dir = manifest_dir.join(p);
            if !dir.exists() {
                return Err(format!(
                    "dependency '{}': path '{}' does not exist",
                    dep.name, p
                ));
            }
            // A path dependency's version is informational (read from its
            // own gx.json if it declares one) — resolution never depends
            // on it, since a path dependency is always used exactly as it
            // sits on disk right now, not fetched or version-matched.
            let version = read_declared_version(&dir).unwrap_or_else(|| "0.0.0".to_string());
            Ok((version, format!("path+{}", p), dir))
        }
        DependencySource::Registry(range) => {
            let resolved = crate::package::resolve_registry_version(cache_root, &dep.name, range)
                .map_err(|e| format!("dependency '{}': {}", dep.name, e))?;
            let Some(version) = resolved else {
                return Err(format!(
                    "dependency '{}': no cached version satisfying \"{}\" was found, and there is \
                     no package registry to fetch one from. Declare it with a 'git' or 'path' \
                     source instead, or install a matching version via another project first \
                     (the local cache at {} is shared across projects on this machine).",
                    dep.name,
                    range,
                    cache_root.display()
                ));
            };
            let dir = crate::package::cache_dir_for(cache_root, &dep.name, &version.to_string());
            Ok((version.to_string(), format!("registry+{}", version), dir))
        }
        DependencySource::Git { url, rev } => {
            fetch_git_dependency(&dep.name, url, rev.as_deref(), cache_root, offline)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_declared_version(dir: &Path) -> Option<String> {
    let manifest_path = dir.join(crate::package::MANIFEST_NAME);
    let content = fs::read_to_string(manifest_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    crate::package::PackageMetadata::from_manifest(&json)
        .ok()?
        .version
        .map(|v| v.to_string())
}

/// Reject a git URL or rev that could be misparsed by `git` itself once it
/// reaches `Command::args` — this is *not* shell-injection defense (`args`
/// already passes each value as its own argv entry, never through a
/// shell), it's defense against `git`'s own argument/transport parsing:
/// - A value starting with `-` would be read as a flag, not a positional
///   argument (classic CLI argument-injection).
/// - Git's "remote helper" syntax (`<name>::<address>`, e.g. `ext::sh -c
///   ...` or `fd::42`) can execute arbitrary commands merely by being
///   passed as a clone URL — this is why `GIT_ALLOW_PROTOCOL` exists
///   upstream. Every legitimate transport (`https://`, `ssh://`,
///   `git://`, `file://`, or scp-like `user@host:path`) uses a single
///   colon or `://`, never the bare `::` a remote-helper name is
///   followed by, so this check has no false positives on real URLs.
#[cfg(not(target_arch = "wasm32"))]
fn validate_git_arg(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("git {} must not be empty", kind));
    }
    if value.starts_with('-') {
        return Err(format!(
            "invalid git {} '{}': must not start with '-' (would be parsed as a flag)",
            kind, value
        ));
    }
    if value.contains("::") {
        return Err(format!(
            "invalid git {} '{}': git \"remote helper\" transports (name::address) are not \
             allowed, since some can execute arbitrary commands",
            kind, value
        ));
    }
    Ok(())
}

/// Clone (or, under `--offline`, reuse whatever is already cached) a git
/// dependency, tagging the cache entry with the package's own declared
/// version if it has one — falling back to `0.0.0+git.<short-sha>` (valid
/// semver build metadata) when it doesn't, so every git dependency still
/// gets a stable, unique cache key even with no version of its own.
#[cfg(not(target_arch = "wasm32"))]
fn fetch_git_dependency(
    name: &str,
    url: &str,
    rev: Option<&str>,
    cache_root: &Path,
    offline: bool,
) -> Result<(String, String, std::path::PathBuf), String> {
    if offline {
        let existing = crate::package::cached_versions(cache_root, name);
        let Some(version) = existing.into_iter().next_back() else {
            return Err(format!(
                "--offline: dependency '{}' is not in the local cache and network access is \
                 disabled. Run `gx install` once without --offline first.",
                name
            ));
        };
        let dir = crate::package::cache_dir_for(cache_root, name, &version.to_string());
        return Ok((version.to_string(), "cached (offline)".to_string(), dir));
    }

    validate_git_arg("url", url).map_err(|e| format!("dependency '{}': {}", name, e))?;
    if let Some(rev) = rev {
        validate_git_arg("rev", rev).map_err(|e| format!("dependency '{}': {}", name, e))?;
    }

    let staging = std::env::temp_dir().join(format!(
        "gx_install_staging_{}_{}",
        std::process::id(),
        sanitize_for_temp_dir(name)
    ));
    let _ = fs::remove_dir_all(&staging);
    let clone_status = Command::new("git")
        .args(["clone", "--quiet", url, &staging.to_string_lossy()])
        .status()
        .map_err(|e| format!("git not found or failed to run ({}). Install git.", e))?;
    if !clone_status.success() {
        return Err(format!(
            "git clone failed for dependency '{}' ({})",
            name, url
        ));
    }
    if let Some(rev) = rev {
        let checkout_status = Command::new("git")
            .args(["-C", &staging.to_string_lossy(), "checkout", "--quiet", rev])
            .status()
            .map_err(|e| format!("git checkout failed: {}", e))?;
        if !checkout_status.success() {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!(
                "git checkout '{}' failed for dependency '{}' ({})",
                rev, name, url
            ));
        }
    }

    let sha_output = Command::new("git")
        .args([
            "-C",
            &staging.to_string_lossy(),
            "rev-parse",
            "--short",
            "HEAD",
        ])
        .output()
        .map_err(|e| format!("git rev-parse failed: {}", e))?;
    let short_sha = String::from_utf8_lossy(&sha_output.stdout)
        .trim()
        .to_string();

    // .git metadata is large, irrelevant to the package's actual content,
    // and would make the integrity hash depend on git-internal state
    // (refs, packfiles) rather than just the source files themselves.
    let _ = fs::remove_dir_all(staging.join(".git"));

    let version =
        read_declared_version(&staging).unwrap_or_else(|| format!("0.0.0+git.{}", short_sha));

    let dest = crate::package::cache_dir_for(cache_root, name, &version);
    let _ = fs::remove_dir_all(&dest);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::rename(&staging, &dest)
        .map_err(|e| format!("failed to move cloned package into the cache: {}", e))?;

    let resolved = match rev {
        Some(r) => format!("git+{}#{}", url, r),
        None => format!("git+{}#{}", url, short_sha),
    };
    Ok((version, resolved, dest))
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitize_for_temp_dir(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

// ── gx publish ───────────────────────────────────────────────────────────────

/// There is no hosted GX package registry (see `crate::package`'s module
/// docs for why that's a deliberate choice, not a gap) — `gx publish`'s job
/// is therefore not to upload anything, but to (1) validate the project is
/// in a state where *other* projects could actually depend on it
/// reproducibly, (2) produce the same integrity manifest `gx install` will
/// later verify against, and (3) tell the developer the honest, real
/// mechanism for distributing it: a git tag, resolved the same way any
/// other `git` dependency already is.
#[cfg(not(target_arch = "wasm32"))]
pub fn publish() -> Result<(), String> {
    let manifest_path = Path::new(crate::package::MANIFEST_NAME);
    if !manifest_path.exists() {
        return Err(format!(
            "No {} found in the current directory.",
            crate::package::MANIFEST_NAME
        ));
    }
    let root = manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let content = fs::read_to_string(manifest_path)
        .map_err(|e| format!("Cannot read {}: {}", crate::package::MANIFEST_NAME, e))?;
    let manifest: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("{} is not valid JSON: {}", crate::package::MANIFEST_NAME, e))?;
    let meta = crate::package::PackageMetadata::from_manifest(&manifest)?;

    let name = meta.name.ok_or_else(|| {
        format!(
            "cannot publish: {} has no \"name\" — every published package needs one so \
             dependents have something to refer to it by",
            crate::package::MANIFEST_NAME
        )
    })?;
    let version = meta.version.ok_or_else(|| {
        format!(
            "cannot publish: {} has no valid semver \"version\" — reproducible builds depend on \
             every published version being pinned to an exact, unambiguous number",
            crate::package::MANIFEST_NAME
        )
    })?;

    // A path dependency only resolves relative to *this* checkout — it is
    // meaningless (and unresolvable) for anyone who depends on this
    // package from outside it, so publishing with one still in place would
    // silently produce a package that's broken for every consumer.
    let deps = crate::package::parse_gx_dependencies(&manifest)?;
    let path_deps: Vec<&str> = deps
        .iter()
        .filter(|d| matches!(d.source, crate::package::DependencySource::Path(_)))
        .map(|d| d.name.as_str())
        .collect();
    if !path_deps.is_empty() {
        return Err(format!(
            "cannot publish '{}': it has path dependencies ({}) which only resolve inside this \
             checkout — switch them to 'git' dependencies (or vendor them) before publishing",
            name,
            path_deps.join(", ")
        ));
    }

    let integrity = crate::package::hash_package_tree(root)
        .map_err(|e| format!("cannot publish '{}': {}", name, e))?;

    let descriptor = serde_json::json!({
        "name": name,
        "version": version.to_string(),
        "entry": meta.entry,
        "integrity": integrity,
    });
    let descriptor_name = format!("{}-{}.gxpkg.json", name, version);
    let descriptor_path = root.join(&descriptor_name);
    let descriptor_json = serde_json::to_string_pretty(&descriptor)
        .map_err(|e| format!("failed to serialize package descriptor: {}", e))?;
    fs::write(&descriptor_path, descriptor_json + "\n")
        .map_err(|e| format!("cannot write '{}': {}", descriptor_path.display(), e))?;

    println!("Wrote {}", descriptor_path.display());
    println!();
    println!("GX has no hosted package registry — packages are distributed via git tags,");
    println!("resolved the same way any 'git' dependency already is:");
    println!();
    println!(
        "  git add {} {}",
        crate::package::MANIFEST_NAME,
        descriptor_name
    );
    println!("  git commit -m \"release {} {}\"", name, version);
    println!("  git tag v{}", version);
    println!("  git push origin main --tags");
    println!();
    println!("Consumers then declare it in gx.json as:");
    println!(
        "  \"{}\": {{ \"git\": \"<this repository's URL>\", \"rev\": \"v{}\" }}",
        name, version
    );
    Ok(())
}

pub fn install(package: &str) -> Result<(), String> {
    // Parse package: js.axios, py.requests, or bare name
    let (namespace, pkg): (&str, &str) = if let Some(rest) = package.strip_prefix("js.") {
        ("js", rest)
    } else if let Some(rest) = package.strip_prefix("py.") {
        ("py", rest)
    } else if let Some(rest) = package.strip_prefix("rust.") {
        ("rust", rest)
    } else {
        // Try to detect from context or default to js
        ("js", package)
    };

    // Validate package name: only allow letters, digits, hyphens, underscores,
    // dots, slashes (scoped npm packages like @org/pkg), and an optional @version.
    // This blocks shell injection and typosquatting attempts with weird chars.
    let name_valid = pkg
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"@/._-".contains(&b));
    // A leading `-` would be read by npm/pip as an option rather than a
    // package name (e.g. a package name of `-g`) — the same CLI
    // argument-injection class `validate_git_arg` already rejects for
    // `git`'s own arguments, applied here too rather than left as a gap
    // specific to this validator.
    if !name_valid || pkg.is_empty() || pkg.starts_with('-') {
        return Err(format!(
            "Invalid package name '{}'. Only alphanumeric, @, /, ., _, - allowed, and it must not start with '-'.",
            pkg
        ));
    }

    match namespace {
        "js" => {
            println!("Installing npm package: {}", pkg);
            let status = Command::new("npm")
                .args(["install", pkg])
                .status()
                .map_err(|_| {
                    "npm not found. Install Node.js from https://nodejs.org".to_string()
                })?;
            if !status.success() {
                return Err(format!("npm install {} failed", pkg));
            }
            update_manifest("js", pkg)?;
            println!("Installed {} (npm)", pkg);
        }
        "py" => {
            println!("Installing Python package: {}", pkg);
            let python =
                find_python().ok_or("Python not found. Install from https://python.org")?;
            let status = Command::new(&python)
                .args(["-m", "pip", "install", pkg])
                .status()
                .map_err(|e| format!("pip failed: {}", e))?;
            if !status.success() {
                return Err(format!("pip install {} failed", pkg));
            }
            update_manifest("py", pkg)?;
            println!("Installed {} (pip)", pkg);
        }
        "rust" => {
            println!(
                "For Rust crate '{}': add to your Cargo.toml and rebuild GX.",
                pkg
            );
            println!("  [dependencies]");
            println!("  {} = \"*\"", pkg);
        }
        _ => return Err(format!("Unknown namespace '{}'", namespace)),
    }

    Ok(())
}

// ── gx fmt ────────────────────────────────────────────────────────────────────

/// `gx fmt <file.gx|dir> [--check]`. `target` is either a single file or a
/// directory (every `.gx` file found recursively, mirroring `gx test`'s
/// and `gx doc`'s own directory-discovery behavior — this used to be the
/// one GX-source-processing command that couldn't operate on a whole
/// project at once). `check` (the CI-friendly convention `cargo fmt
/// --check`/`prettier --check` already established) reports which files
/// would change without writing anything, and returns an error (so the
/// process exits non-zero) if any would — `gx fmt` itself keeps writing
/// in place, unchanged from before.
pub fn fmt(target: &str, check: bool) -> Result<(), String> {
    let target_path = Path::new(target);
    if !target_path.exists() {
        return Err(format!("'{}' not found", target));
    }

    let mut files = Vec::new();
    if target_path.is_file() {
        files.push(target.to_string());
    } else {
        collect_gx_files(target_path, &mut files);
    }
    files.sort();
    if files.is_empty() {
        return Err(format!("no .gx files found in '{}'", target));
    }

    let mut unformatted = Vec::new();
    for file in &files {
        let source =
            fs::read_to_string(file).map_err(|e| format!("Cannot read {}: {}", file, e))?;
        let formatted = format_source(&source).map_err(|e| format!("{}: {}", file, e))?;

        if check {
            if formatted != source {
                unformatted.push(file.clone());
            }
        } else if formatted != source {
            fs::write(file, &formatted).map_err(|e| format!("Cannot write {}: {}", file, e))?;
            println!("Formatted: {}", file);
        }
    }

    if check {
        if unformatted.is_empty() {
            println!(
                "{} file{} already formatted",
                files.len(),
                if files.len() == 1 { "" } else { "s" }
            );
            Ok(())
        } else {
            for f in &unformatted {
                println!("would reformat: {}", f);
            }
            Err(format!(
                "{} file{} would be reformatted (run `gx fmt` without --check to write them)",
                unformatted.len(),
                if unformatted.len() == 1 { "" } else { "s" }
            ))
        }
    } else {
        Ok(())
    }
}

/// The pure "source text in, formatted text out" core — no file I/O, so
/// `gx fmt --check` can compare without ever writing, and `gx fmt` on a
/// directory can call this once per file without duplicating the
/// indent-vs-brace-syntax branching.
fn format_source(source: &str) -> Result<String, String> {
    use crate::indent_parser::is_indent_syntax;
    use crate::lexer::{Lexer, TokenKind};

    // Progressive syntax: normalize indentation instead of token-based reformatting
    if is_indent_syntax(source) {
        return Ok(fmt_indent_syntax(source));
    }

    let mut lexer = Lexer::new(source);
    let tokens = lexer
        .tokenize()
        .map_err(|e| format!("Syntax error: {}", e))?;

    let mut output = String::new();
    let mut indent = 0usize;
    let mut last_was_newline = false;
    let mut prev_kind: Option<TokenKind> = None;

    for token in &tokens {
        match &token.kind {
            TokenKind::Eof => break,
            TokenKind::Newline => {
                if !last_was_newline {
                    output.push('\n');
                    last_was_newline = true;
                }
            }
            TokenKind::LBrace => {
                if !output.ends_with(' ') && !output.ends_with('\n') {
                    output.push(' ');
                }
                output.push('{');
                indent += 1;
                output.push('\n');
                last_was_newline = true;
            }
            TokenKind::RBrace => {
                indent = indent.saturating_sub(1);
                if !last_was_newline {
                    output.push('\n');
                }
                output.push_str(&"    ".repeat(indent));
                output.push('}');
                output.push('\n');
                last_was_newline = true;
            }
            other => {
                if last_was_newline {
                    output.push_str(&"    ".repeat(indent));
                } else if needs_space_before(prev_kind.as_ref(), other) {
                    output.push(' ');
                }
                output.push_str(&token_to_str(other));
                prev_kind = Some(other.clone());
                last_was_newline = false;
            }
        }
    }

    Ok(output.trim().to_string() + "\n")
}

/// Whether a space belongs between `prev` (the last token actually written,
/// `None` at the start of a line — indentation already covers that case)
/// and `next` (the token about to be written). Conventional formatters
/// (Prettier, rustfmt, gofmt) don't pad every delimiter uniformly — `f(x)`
/// and `arr[i]`, not `f ( x )` and `arr [ i ]` — and gx fmt's previous
/// unconditional "one space after every token" rule made dense,
/// deeply-nested calls (which real GX code has plenty of, e.g. every SQL
/// query call) noticeably harder to scan than in any comparable language.
fn needs_space_before(
    prev: Option<&crate::lexer::TokenKind>,
    next: &crate::lexer::TokenKind,
) -> bool {
    use crate::lexer::TokenKind::*;
    // These never take a leading space, regardless of what precedes them:
    // `)`, `]`, `,`, `.`, and object/param `key:` never want a space
    // before them.
    if matches!(next, RParen | RBracket | Comma | Dot | Colon) {
        return false;
    }
    match prev {
        None => false,
        // Nothing wants a space directly after an opening delimiter or a
        // `.` (method/property chaining).
        Some(LParen) | Some(LBracket) | Some(Dot) => false,
        // Call/subscript syntax: `f(x)`, `arr[i]` — no space between the
        // callee/target and its opening delimiter. `)`/`]` also chain
        // this way for `f(x)(y)`/`arr[i][j]`-shaped expressions.
        Some(Ident(_)) | Some(RParen) | Some(RBracket) if matches!(next, LParen | LBracket) => {
            false
        }
        _ => true,
    }
}

fn token_to_str(kind: &crate::lexer::TokenKind) -> String {
    use crate::lexer::TokenKind::*;
    match kind {
        Ident(s) => s.clone(),
        // The lexer's `read_string` *decodes* \\, \n, \t, \" while
        // tokenizing (see Lexer::read_string), so a StringLit's value
        // already contains the real characters, not the escape
        // sequences — re-emitting it verbatim (as a previous version of
        // this function did, only re-escaping `"`) wrote literal newline/
        // tab bytes into the formatted source, silently turning a
        // one-line string into a multi-line one and making formatting
        // non-idempotent (a second `gx fmt` pass would see different
        // content and reformat again). Escaping backslashes first is
        // required — doing any of the others first would then have its
        // own inserted backslash re-escaped.
        StringLit(s) => format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
        ),
        NumberLit(n) => {
            if n.fract() == 0.0 {
                format!("{}", *n as i64)
            } else {
                format!("{}", n)
            }
        }
        BoolLit(b) => b.to_string(),
        Null => "null".into(),
        Helper => "helper".into(),
        Agent => "agent".into(),
        Brain => "brain".into(),
        Plan => "plan".into(),
        Execute => "execute".into(),
        Remember => "remember".into(),
        Communicate => "communicate".into(),
        Memory => "memory".into(),
        Receive => "receive".into(),
        Channel => "channel".into(),
        Broadcast => "broadcast".into(),
        Objective => "objective".into(),
        Needs => "needs".into(),
        Gives => "gives".into(),
        CanDo => "can_do".into(),
        When => "when".into(),
        Then => "then".into(),
        Started => "started".into(),
        ReRun => "re-run".into(),
        Escalate => "escalate".into(),
        Human => "human".into(),
        Changes => "changes".into(),
        Ask => "ask".into(),
        Embed => "embed".into(),
        Infer => "infer".into(),
        Classifier => "classifier".into(),
        Use => "use".into(),
        From => "from".into(),
        As => "as".into(),
        On => "on".into(),
        Bind => "bind".into(),
        Source => "source".into(),
        Type => "type".into(),
        Do => "do".into(),
        Wait => "wait".into(),
        Assign => "assign".into(),
        Spawn => "spawn".into(),
        Count => "count".into(),
        Push => "push".into(),
        If => "if".into(),
        Else => "else".into(),
        For => "for".into(),
        Each => "each".into(),
        In => "in".into(),
        Try => "try".into(),
        Catch => "catch".into(),
        Say => "say".into(),
        Log => "log".into(),
        Output => "output".into(),
        Return => "return".into(),
        Emit => "emit".into(),
        Recipe => "recipe".into(),
        // `token_to_str` can't tell a named `function foo(...)` apart from
        // an anonymous `fn(...)` closure literal — both spellings collapse
        // to the same `TokenKind::Function` in the lexer, and this
        // function only sees one token at a time. Re-emitting "function"
        // uniformly is always valid syntax for both (the parser treats
        // them identically too), even though it doesn't preserve which
        // spelling the original source used for a closure.
        Function => "function".into(),
        Import => "import".into(),
        While => "while".into(),
        Break => "break".into(),
        Continue => "continue".into(),
        Assert => "assert".into(),
        Serve => "serve".into(),
        Route => "route".into(),
        Respond => "respond".into(),
        Port => "port".into(),
        With => "with".into(),
        To => "to".into(),
        Message => "message".into(),
        Call => "call".into(),
        Pipe => "|>".into(),
        Goal => "goal".into(),
        Think => "think".into(),
        Act => "act".into(),
        Observe => "observe".into(),
        Loop => "loop".into(),
        Until => "until".into(),
        Repeat => "repeat".into(),
        Times => "times".into(),
        Parallel => "parallel".into(),
        Retry => "retry".into(),
        Timeout => "timeout".into(),
        OnError => "on_error".into(),
        Cron => "cron".into(),
        Tool => "tool".into(),
        Schema => "schema".into(),
        Await => "await".into(),
        Required => "required".into(),
        Description => "description".into(),
        Persistent => "persistent".into(),
        Colon => ":".into(),
        Comma => ",".into(),
        Dot => ".".into(),
        Eq => "=".into(),
        EqEq => "==".into(),
        NotEq => "!=".into(),
        Lt => "<".into(),
        LtEq => "<=".into(),
        Gt => ">".into(),
        GtEq => ">=".into(),
        Plus => "+".into(),
        PlusEq => "+=".into(),
        Minus => "-".into(),
        MinusEq => "-=".into(),
        Star => "*".into(),
        StarEq => "*=".into(),
        Slash => "/".into(),
        SlashEq => "/=".into(),
        Percent => "%".into(),
        Arrow => "->".into(),
        DotDot => "..".into(),
        QuestionQuestion => "??".into(),
        LParen => "(".into(),
        RParen => ")".into(),
        LBracket => "[".into(),
        RBracket => "]".into(),
        And => "and".into(),
        Or => "or".into(),
        Not => "not".into(),
        // Handled by the caller's own loop (Newline flushes a line,
        // LBrace/RBrace manage indentation) — never reached via this
        // function, but included so the match stays exhaustive (no `_`
        // wildcard) and a future TokenKind variant that's genuinely
        // missed here is a compile error, not a silently-deleted token
        // the way every keyword above this fix used to be.
        LBrace | RBrace | Newline | Eof => String::new(),
    }
}

// ── gx doc ────────────────────────────────────────────────────────────────────

/// Generate a Markdown API reference from a `.gx` file or directory:
/// every `function`, `agent`/`helper`, and `tool` definition, its
/// signature, and (functions/agents) any `//`-comment block immediately
/// preceding its declaration in the source — the same "doc comment"
/// convention most languages use, except GX's lexer discards comments
/// during tokenization, so this reads the *raw source text* directly
/// rather than the parsed AST for that part. Tools are self-documenting
/// already (`description`/per-parameter `description` are real AST
/// fields, not comments), so those are used verbatim.
pub fn doc(target: &str, out: Option<&str>) -> Result<(), String> {
    use crate::indent_parser::is_indent_syntax;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    let target_path = Path::new(target);
    if !target_path.exists() {
        return Err(format!("'{}' not found", target));
    }

    let mut files = Vec::new();
    if target_path.is_file() {
        files.push(target.to_string());
    } else {
        collect_gx_files(target_path, &mut files);
    }
    files.sort();
    if files.is_empty() {
        return Err(format!("no .gx files found in '{}'", target));
    }

    let mut md = String::new();
    md.push_str(&format!("# GX API Reference: {}\n\n", target));
    md.push_str("_Generated by `gx doc` — do not edit by hand._\n\n");

    for file in &files {
        let source =
            fs::read_to_string(file).map_err(|e| format!("cannot read {}: {}", file, e))?;
        let program = if is_indent_syntax(&source) {
            crate::indent_parser::parse(&source).map_err(|e| format!("{}: {}", file, e))?
        } else {
            let tokens = Lexer::new(&source)
                .tokenize()
                .map_err(|e| format!("{}: {}", file, e))?;
            Parser::new(tokens)
                .parse()
                .map_err(|e| format!("{}: {}", file, e))?
        };

        if program.functions.is_empty() && program.helpers.is_empty() && program.tools.is_empty() {
            continue;
        }

        md.push_str(&format!("## {}\n\n", file));

        if !program.functions.is_empty() {
            md.push_str("### Functions\n\n");
            let mut fns = program.functions.clone();
            fns.sort_by(|a, b| a.name.cmp(&b.name));
            for f in &fns {
                md.push_str(&format!("#### `{}({})`\n\n", f.name, f.params.join(", ")));
                if let Some(doc_comment) = extract_doc_comment(&source, f.line) {
                    md.push_str(&doc_comment);
                    md.push_str("\n\n");
                }
                md.push_str(&format!("_defined at {}:{}_\n\n", file, f.line));
            }
        }

        if !program.helpers.is_empty() {
            md.push_str("### Agents\n\n");
            let mut helpers = program.helpers.clone();
            helpers.sort_by(|a, b| a.name.cmp(&b.name));
            for h in &helpers {
                md.push_str(&format!("#### `{}`\n\n", h.name));
                if let Some(goal) = &h.goal {
                    md.push_str(&format!("Goal: {}\n\n", goal));
                }
                if let Some(doc_comment) = extract_doc_comment(&source, h.line) {
                    md.push_str(&doc_comment);
                    md.push_str("\n\n");
                }
                md.push_str(&format!("_defined at {}:{}_\n\n", file, h.line));
            }
        }

        if !program.tools.is_empty() {
            md.push_str("### Tools\n\n");
            let mut tools = program.tools.clone();
            tools.sort_by(|a, b| a.name.cmp(&b.name));
            for t in &tools {
                let params: Vec<String> = t
                    .params
                    .iter()
                    .map(|p| {
                        format!(
                            "{}{}: {}",
                            p.name,
                            if p.required { "" } else { "?" },
                            p.param_type
                        )
                    })
                    .collect();
                md.push_str(&format!("#### `{}({})`\n\n", t.name, params.join(", ")));
                if !t.description.is_empty() {
                    md.push_str(&format!("{}\n\n", t.description));
                }
                for p in &t.params {
                    if let Some(pd) = &p.description {
                        md.push_str(&format!("- `{}`: {}\n", p.name, pd));
                    }
                }
                if t.params.iter().any(|p| p.description.is_some()) {
                    md.push('\n');
                }
                md.push_str(&format!("_defined at {}:{}_\n\n", file, t.line));
            }
        }
    }

    match out {
        Some(path) => {
            fs::write(path, &md).map_err(|e| format!("cannot write {}: {}", path, e))?;
            println!("Wrote {}", path);
        }
        None => print!("{}", md),
    }
    Ok(())
}

/// Look backward from `def_line` (1-based, the line a function/agent
/// declaration starts on) through consecutive `//`-comment lines,
/// stopping at the first blank or non-comment line. Returns the comment
/// text (leading `//` and one following space stripped from each line,
/// joined with newlines), or `None` if there's no comment immediately
/// above the declaration.
fn extract_doc_comment(source: &str, def_line: usize) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    if def_line < 2 {
        return None;
    }
    let mut comment_lines = Vec::new();
    // def_line is 1-based; the line immediately above it is index def_line - 2.
    let mut i = def_line.checked_sub(2)?;
    loop {
        let trimmed = lines.get(i)?.trim();
        if let Some(rest) = trimmed.strip_prefix("//") {
            comment_lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
            if i == 0 {
                break;
            }
            i -= 1;
        } else {
            break;
        }
    }
    if comment_lines.is_empty() {
        return None;
    }
    comment_lines.reverse();
    Some(comment_lines.join("\n"))
}

// ── gx make ───────────────────────────────────────────────────────────────────

pub fn make(input: &str, out_dir: Option<&str>) -> Result<(), String> {
    use crate::ai;
    use crate::value::Value;
    use std::collections::HashMap;

    // Input is either a .gx spec file or a plain text description
    let spec = if input.ends_with(".gx") && Path::new(input).exists() {
        let raw = fs::read_to_string(input).map_err(|e| format!("Cannot read {}: {}", input, e))?;
        println!("Reading spec: {}", input);
        raw
    } else {
        input.to_string()
    };

    println!("Generating project with AI...");

    let system_prompt = r#"You are an expert software architect and full-stack developer.
Your job is to generate complete, working software projects from specifications.

RULES:
- Return ONLY a valid JSON object — no markdown fences, no explanation, no text before or after
- Every file must contain complete, working code — no placeholder comments like "// TODO" or "// implement me"
- Code must run without modification after following the setup commands
- Prefer simplicity: use the fewest dependencies that get the job done
- Include a helpful README.md in every project

JSON SCHEMA (respond with exactly this structure):
{
  "project_name": "kebab-case-directory-name",
  "description": "one sentence describing what this project does",
  "files": [
    {
      "path": "relative/path/to/file.ext",
      "content": "complete file content as a string"
    }
  ],
  "setup": ["command1", "command2"],
  "run": "command to start the project",
  "notes": "any important notes for the user"
}"#;

    let user_prompt = format!(
        "Generate a complete, working project for the following specification:\n\n{}\n\nReturn only the JSON object.",
        spec.trim()
    );

    let mut params = HashMap::new();
    params.insert("prompt".into(), Value::Str(user_prompt));
    params.insert("system".into(), Value::Str(system_prompt.into()));
    params.insert("max_tokens".into(), Value::Number(4000.0));
    params.insert("temperature".into(), Value::Number(0.2));

    // Try providers in order: openai → anthropic → ollama
    // Use env var GX_PROVIDER to override
    let env_provider = std::env::var("GX_PROVIDER").unwrap_or_default();
    let providers: Vec<(&str, Option<&str>)> = if !env_provider.is_empty() {
        vec![(Box::leak(env_provider.into_boxed_str()), None)]
    } else {
        vec![
            ("openai", Some("gpt-4o-mini")),
            ("anthropic", Some("claude-haiku-4-5")),
            ("ollama", Some("llama3.2:latest")),
        ]
    };
    let mut ai_text = String::new();
    let mut last_error = "No AI provider configured.".to_string();
    // `gx make` is a one-shot CLI invocation with no running Interpreter (and
    // so no `self.http_agent()`/capability-checked agent to reuse) — a
    // plain default agent is fine here; the point of reusing an agent in
    // the interpreter is connection pooling *across* many calls in one
    // script run, which doesn't apply to a single generate-and-exit command.
    // `gx make` is a CLI-only command (writes files, runs setup commands) —
    // not reachable from the WASM playground build at all, so `ureq`
    // (unavailable under wasm32) is only constructed on native targets.
    #[cfg(not(target_arch = "wasm32"))]
    let agent = ureq::agent();

    for (provider, default_model) in &providers {
        let env_model = std::env::var("GX_MODEL").unwrap_or_default();
        let model = if !env_model.is_empty() {
            Some(env_model.as_str())
        } else {
            *default_model
        };
        #[cfg(not(target_arch = "wasm32"))]
        let result = ai::ask_ai(provider, model, &params, &agent);
        #[cfg(target_arch = "wasm32")]
        let result = ai::ask_ai(provider, model, &params);
        if let Value::Object(ref map) = result {
            if map.get("ok") == Some(&Value::Bool(true)) {
                if let Some(Value::Str(text)) = map.get("text") {
                    ai_text = text.trim().to_string();
                    println!("  (generated with {} {})", provider, model.unwrap_or(""));
                    break;
                }
            } else if let Some(Value::Str(err)) = map.get("error") {
                last_error = err.clone();
            }
        }
    }

    if ai_text.is_empty() {
        return Err(format!(
            "Could not generate project.\n{}\n\nSet OPENAI_API_KEY or ANTHROPIC_API_KEY.",
            last_error
        ));
    }

    // Strip markdown fences the AI sometimes adds despite instructions
    let json_str = strip_json_fences(&ai_text);

    // Parse the JSON response (try raw first, then repair unescaped control chars)
    let json: serde_json::Value = serde_json::from_str(json_str)
        .or_else(|_| {
            let repaired = repair_json(json_str);
            serde_json::from_str(&repaired)
        })
        .map_err(|e| {
            format!(
                "AI returned invalid JSON: {}\n\nRaw response:\n{}",
                e,
                truncate_at_char_boundary(&ai_text, 500)
            )
        })?;

    let project_name = json["project_name"]
        .as_str()
        .unwrap_or("gx-project")
        .to_string();
    let description = json["description"].as_str().unwrap_or("").to_string();
    let run_cmd = json["run"].as_str().unwrap_or("").to_string();
    let notes = json["notes"].as_str().unwrap_or("").to_string();

    // Determine output directory
    let target_dir = out_dir.unwrap_or(&project_name);
    let target_path = Path::new(target_dir);

    if target_path.exists() {
        return Err(format!(
            "Directory '{}' already exists. Use --out <other-name> to choose a different name.",
            target_dir
        ));
    }

    // Write all files
    let files = json["files"]
        .as_array()
        .ok_or("JSON missing 'files' array")?;
    if files.is_empty() {
        return Err("AI returned no files.".into());
    }

    println!();
    println!("Creating project: {}", target_dir);
    if !description.is_empty() {
        println!("  {}", description);
    }
    println!();

    for file_entry in files {
        let rel_path = file_entry["path"]
            .as_str()
            .ok_or("File entry missing 'path'")?;
        let content = file_entry["content"].as_str().unwrap_or("");

        let full_path = target_path.join(rel_path);

        // Create parent directories
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create directory {:?}: {}", parent, e))?;
        }

        fs::write(&full_path, content)
            .map_err(|e| format!("Cannot write {:?}: {}", full_path, e))?;

        println!("  wrote  {}/{}", target_dir, rel_path);
    }

    // Setup commands
    let setup_cmds: Vec<String> = json["setup"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    println!();
    println!("Done! {} file(s) written to {}/", files.len(), target_dir);
    println!();

    if !setup_cmds.is_empty() || !run_cmd.is_empty() {
        println!("Next steps:");
        println!("  cd {}", target_dir);
        for cmd in &setup_cmds {
            println!("  {}", cmd);
        }
        if !run_cmd.is_empty() {
            println!("  {}", run_cmd);
        }
        println!();
    }

    if !notes.is_empty() {
        println!("Notes: {}", notes);
        println!();
    }

    Ok(())
}

/// Escape unescaped control characters inside JSON string values.
/// LLMs often emit literal newlines inside strings instead of \n.
fn repair_json(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 64);
    let mut in_string = false;
    let mut escaped = false;
    for ch in s.chars() {
        if escaped {
            escaped = false;
            result.push(ch);
            continue;
        }
        if ch == '\\' && in_string {
            escaped = true;
            result.push(ch);
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            result.push(ch);
            continue;
        }
        if in_string {
            match ch {
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                '\x00'..='\x1f' => {
                    result.push_str(&format!("\\u{:04x}", ch as u32));
                }
                _ => result.push(ch),
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn strip_json_fences(s: &str) -> &str {
    let s = s.trim();
    // Strip ```json ... ``` or ``` ... ```
    if s.starts_with("```") {
        let after_fence = s.trim_start_matches('`');
        // Skip the language identifier if present (e.g. "json\n")
        let after_lang = if let Some(newline) = after_fence.find('\n') {
            &after_fence[newline + 1..]
        } else {
            after_fence
        };
        after_lang
            .trim_end_matches('`')
            .trim_end_matches('\n')
            .trim()
    } else {
        s
    }
}

// ── gx test ───────────────────────────────────────────────────────────────────

pub fn test(path: Option<&str>) -> Result<(), String> {
    use crate::indent_parser::is_indent_syntax;
    use crate::interpreter::{Env, Interpreter};
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    // Resolve the test target:
    //   - explicit single .gx file → run just that file
    //   - explicit directory → discover test files within it
    //   - no argument → try "tests/", then current directory
    let target = path.unwrap_or_else(|| {
        if Path::new("tests").exists() {
            "tests"
        } else {
            "."
        }
    });

    let target_path = Path::new(target);
    if !target_path.exists() {
        return Err(format!("Test path '{}' not found", target));
    }

    let mut test_files = Vec::new();
    if target_path.is_file() {
        test_files.push(target.to_string());
    } else {
        collect_test_files(target_path, &mut test_files);
        // If no test_*.gx / *.test.gx files matched, fall back to all .gx files
        if test_files.is_empty() {
            collect_gx_files(target_path, &mut test_files);
        }
    }
    test_files.sort();

    if test_files.is_empty() {
        println!("No .gx test files found in '{}'", target);
        return Ok(());
    }

    println!("Running {} test file(s)...\n", test_files.len());

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut errors = 0usize;
    let mut total_asserts = 0usize;
    let mut failed_asserts: Vec<String> = Vec::new();

    for file in &test_files {
        let source = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  ERROR  {}: {}", file, e);
                errors += 1;
                continue;
            }
        };

        let program = if is_indent_syntax(&source) {
            match crate::indent_parser::parse(&source) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("  ERROR  {}: {}", file, e);
                    errors += 1;
                    continue;
                }
            }
        } else {
            let tokens = match Lexer::new(&source).tokenize() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("  ERROR  {}: {}", file, e);
                    errors += 1;
                    continue;
                }
            };
            match Parser::new(tokens).parse() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("  ERROR  {}: {}", file, e);
                    errors += 1;
                    continue;
                }
            }
        };

        let mut interp = Interpreter::new();
        interp.base_path = Some(file.to_string());
        // `gx test` runs trusted local test files, so exercise the real
        // capability-gated code path rather than forcing every process-
        // runtime test to special-case a missing --allow-process flag.
        // Shell/internal-network stay at their safe defaults (denied) —
        // no test file needs them, and granting capabilities no test
        // exercises would silently mask a missing check elsewhere.
        interp.capabilities.process = true;

        match interp.run_program(&program) {
            Ok(_) => {
                total_asserts += interp.assert_count;
                if !interp.assert_failures.is_empty() {
                    for af in &interp.assert_failures {
                        eprintln!("  FAIL   {} — {}", file, af);
                        failed_asserts.push(format!("{}: {}", file, af));
                    }
                    failed += 1;
                } else {
                    let assert_note = if interp.assert_count > 0 {
                        format!(" ({} assertions)", interp.assert_count)
                    } else {
                        String::new()
                    };
                    println!("  PASS   {}{}", file, assert_note);
                    passed += 1;
                }

                // Production Testing Framework: every `test(name, fn)` case
                // registered while the script above ran gets executed here,
                // separately and in isolation — its own before_each/
                // after_each, its own fresh assert count/failure list — and
                // reported under its own name rather than folded into the
                // file-level PASS/FAIL above. A file that registers no
                // tests (the common case today, and every pre-existing
                // `tests/*.gx` file) sees no change at all: `registered` is
                // simply empty.
                let registered = interp.take_registered_tests();
                let before_each = interp.before_each_hook();
                let after_each = interp.after_each_hook();
                for (name, test_fn) in registered {
                    let mut test_ok = true;
                    let mut messages: Vec<String> = Vec::new();
                    // One Env shared across before_each → the test body →
                    // after_each *for this one test case* — the channel
                    // that lets before_each hand state to the test via
                    // `memory.*` (see `call_registered_closure`'s doc
                    // comment for why a plain captured variable can't do
                    // this). Fresh per test case, so one test's leftover
                    // memory never leaks into the next.
                    let mut test_env = Env::new();

                    if let Some(hook) = &before_each {
                        if let Err(e) = interp.call_registered_closure(hook, &mut test_env) {
                            test_ok = false;
                            messages.push(format!("before_each: {}", e));
                        }
                    }

                    if test_ok {
                        interp.assert_count = 0;
                        interp.assert_failures.clear();
                        let result = interp.call_registered_closure(&test_fn, &mut test_env);
                        total_asserts += interp.assert_count;
                        // An assertion failure is *both* recorded in
                        // `assert_failures` *and* returned as `result`'s
                        // Err (via Signal::AssertFail) — the same event
                        // wrapped two ways, not two separate failures.
                        // `assert_failures` wins when present (it's the
                        // raw, unwrapped message the script wrote); `result`
                        // is only consulted for a failure that has *no*
                        // matching assert_failures entry — a thrown,
                        // non-assertion runtime error.
                        if !interp.assert_failures.is_empty() {
                            test_ok = false;
                            for af in &interp.assert_failures {
                                messages.push(af.clone());
                            }
                        } else if let Err(e) = &result {
                            test_ok = false;
                            messages.push(e.clone());
                        }
                    }

                    // Teardown always gets a chance to run, even when
                    // before_each or the test body itself failed — the
                    // same "cleanup runs regardless" shape a `finally`
                    // block has, so a test that partially set up state
                    // before failing doesn't leak it into the next test.
                    if let Some(hook) = &after_each {
                        if let Err(e) = interp.call_registered_closure(hook, &mut test_env) {
                            test_ok = false;
                            messages.push(format!("after_each: {}", e));
                        }
                    }

                    if test_ok {
                        let note = if interp.assert_count > 0 {
                            format!(" ({} assertions)", interp.assert_count)
                        } else {
                            String::new()
                        };
                        println!("  PASS   {} :: {}{}", file, name, note);
                        passed += 1;
                    } else {
                        for m in &messages {
                            eprintln!("  FAIL   {} :: {} — {}", file, name, m);
                            failed_asserts.push(format!("{} :: {}: {}", file, name, m));
                        }
                        failed += 1;
                    }
                }
            }
            Err(e) => {
                total_asserts += interp.assert_count;
                eprintln!("  FAIL   {}: {}", file, e);
                failed += 1;
            }
        }
    }

    println!();
    println!(
        "Results: {} passed, {} failed, {} errors | {} total assertions",
        passed, failed, errors, total_asserts
    );

    if failed > 0 || errors > 0 {
        Err(format!("{} test(s) failed", failed + errors))
    } else {
        Ok(())
    }
}

fn collect_gx_files(dir: &Path, files: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_gx_files(&path, files);
            } else if path.extension().map(|e| e == "gx").unwrap_or(false) {
                if let Some(s) = path.to_str() {
                    files.push(s.to_string());
                }
            }
        }
    }
}

/// Discover test files by convention: `test_*.gx` or `*.test.gx`.
fn collect_test_files(dir: &Path, files: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_test_files(&path, files);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                let is_test = (name.starts_with("test_") && name.ends_with(".gx"))
                    || name.ends_with(".test.gx");
                if is_test {
                    if let Some(s) = path.to_str() {
                        files.push(s.to_string());
                    }
                }
            }
        }
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn fmt_indent_syntax(source: &str) -> String {
    // Normalize progressive syntax: standardize indentation to 2 spaces
    let mut result = String::new();
    let mut prev_blank = false;
    for line in source.lines() {
        // `trim_end()` only strips *trailing* whitespace — the line's
        // original leading indentation was still part of the resulting
        // string, so it got written a second time right after the freshly
        // computed indent below. That made every `gx fmt` pass on a
        // progressive-syntax file grow its indentation instead of
        // normalizing it: a second pass would see the doubled indent from
        // the first, compute an even deeper level from it, and double it
        // again — `gx fmt --check` right after `gx fmt` itself would then
        // report the file as needing to be reformatted again.
        let stripped_start = line.trim_start();
        let trimmed = stripped_start.trim_end();
        if trimmed.is_empty() {
            if !prev_blank {
                result.push('\n');
                prev_blank = true;
            }
            continue;
        }
        prev_blank = false;
        // Count existing indentation
        let indent_count = line.len() - stripped_start.len();
        // Normalize: each 4-space or 1-tab indent level becomes 2 spaces
        let level = if indent_count > 0 {
            indent_count.div_ceil(2)
        } else {
            0
        };
        let indent = "  ".repeat(level);
        result.push_str(&indent);
        result.push_str(trimmed);
        result.push('\n');
    }
    result
}

fn find_python() -> Option<String> {
    for candidate in &["python3", "python"] {
        if Command::new(if cfg!(windows) { "where" } else { "which" })
            .arg(candidate)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some((*candidate).into());
        }
    }
    None
}

fn update_manifest(namespace: &str, package: &str) -> Result<(), String> {
    let manifest_path = Path::new("gx.json");
    if !manifest_path.exists() {
        return Ok(());
    }

    let content =
        fs::read_to_string(manifest_path).map_err(|e| format!("Cannot read gx.json: {}", e))?;

    // Simple string replacement — don't want serde_json complexity for this
    let dep_key = format!("\"{}\":", namespace);
    if let Some(pos) = content.find(&dep_key) {
        if let Some(bracket_pos) = content[pos..].find('[') {
            let insert_at = pos + bracket_pos + 1;
            let new_entry = format!("\"{}\"", package);
            // Check if already present
            if content.contains(&new_entry) {
                return Ok(());
            }
            let mut new_content = content.clone();
            new_content.insert_str(insert_at, &format!("{}, ", new_entry));
            fs::write(manifest_path, &new_content)
                .map_err(|e| format!("Cannot write gx.json: {}", e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod build_tests {
    use super::*;

    #[test]
    fn pick_heredoc_delimiter_skips_a_candidate_that_collides_with_a_source_line() {
        // Regression test for a heredoc-injection bug: `build()` used to
        // embed `.gx` source into a shell heredoc with a fixed delimiter
        // (`__GX_SOURCE_EOF__`), so a source file containing a line equal
        // to that exact delimiter would terminate the heredoc early and
        // turn everything after it into literal, unsandboxed shell script.
        let source = "echo hi\nCOLLIDES\nsay \"done\"\n";
        let candidates = vec!["COLLIDES".to_string(), "UNIQUE_OK".to_string()].into_iter();
        let d = pick_heredoc_delimiter(source, candidates);
        assert_eq!(d, "UNIQUE_OK");
    }

    #[test]
    fn pick_heredoc_delimiter_accepts_the_first_candidate_when_it_does_not_collide() {
        let source = "say \"hello\"\n";
        let candidates = vec!["FINE".to_string()].into_iter();
        let d = pick_heredoc_delimiter(source, candidates);
        assert_eq!(d, "FINE");
    }

    #[test]
    fn truncate_at_char_boundary_does_not_panic_when_max_lands_inside_a_multi_byte_char() {
        // Regression test: `&ai_text[..500]`-style raw byte slicing used to
        // panic ("byte index is not a char boundary") whenever an AI
        // response contained a multi-byte UTF-8 character straddling the
        // cutoff — reachable from `gx make`'s error-formatting path on any
        // AI response with a curly quote/em dash near that offset.
        let s = "ab\u{e9}cdef";
        assert_eq!(truncate_at_char_boundary(s, 3), "ab");
    }

    #[test]
    fn truncate_at_char_boundary_leaves_short_strings_unchanged() {
        assert_eq!(truncate_at_char_boundary("hi", 10), "hi");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod install_tests {
    use super::*;
    use crate::package::{Dependency, DependencySource};

    // Every helper below takes `cache_root` as an explicit parameter rather
    // than reading `GX_PACKAGE_CACHE_DIR` — this lets tests run under
    // cargo test's default parallelism without racing each other or
    // `package.rs`'s own tests over shared global env state (the same
    // hazard already fixed once in package.rs; see its module docs).

    fn temp_dir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "gx_toolchain_test_{}_{}_{}",
            label,
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git must be installed to run this test");
        assert!(status.success(), "git {:?} failed in {:?}", args, dir);
    }

    fn make_git_repo(label: &str, gx_json: Option<&str>) -> std::path::PathBuf {
        let dir = temp_dir(label);
        fs::write(dir.join("main.gx"), "function f() { return 1 }\n").unwrap();
        if let Some(json) = gx_json {
            fs::write(dir.join("gx.json"), json).unwrap();
        }
        git(&dir, &["init", "-q"]);
        git(&dir, &["add", "-A"]);
        git(
            &dir,
            &[
                "-c",
                "user.email=t@t.com",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "init",
            ],
        );
        dir
    }

    #[test]
    fn resolve_path_dependency_reads_the_target_and_its_declared_version() {
        let target = temp_dir("path-target");
        fs::write(
            target.join("gx.json"),
            r#"{"name":"pathdep","version":"2.5.0","entry":"main.gx"}"#,
        )
        .unwrap();
        let manifest_dir = temp_dir("path-consumer");
        let cache_root = temp_dir("path-cache");
        let dep = Dependency {
            name: "pathdep".to_string(),
            source: DependencySource::Path(
                "../".to_string() + target.file_name().unwrap().to_str().unwrap(),
            ),
        };
        // manifest_dir and target must be siblings for the relative path to work
        let target_parent = target.parent().unwrap();
        let manifest_dir = target_parent.join(manifest_dir.file_name().unwrap());
        fs::create_dir_all(&manifest_dir).unwrap();

        let (version, resolved, dir) =
            resolve_one_dependency(&dep, &manifest_dir, &cache_root, false).unwrap();
        assert_eq!(version, "2.5.0");
        assert!(resolved.starts_with("path+"));
        assert_eq!(dir.canonicalize().unwrap(), target.canonicalize().unwrap());
    }

    #[test]
    fn resolve_path_dependency_errors_clearly_when_the_target_is_missing() {
        let manifest_dir = temp_dir("path-missing-consumer");
        let cache_root = temp_dir("path-missing-cache");
        let dep = Dependency {
            name: "ghost".to_string(),
            source: DependencySource::Path("does-not-exist".to_string()),
        };
        let err = resolve_one_dependency(&dep, &manifest_dir, &cache_root, false).unwrap_err();
        assert!(
            err.contains("ghost"),
            "error should name the dependency: {}",
            err
        );
        assert!(
            err.contains("does not exist"),
            "error should say why: {}",
            err
        );
    }

    #[test]
    fn resolve_registry_dependency_errors_clearly_when_nothing_is_cached() {
        let manifest_dir = temp_dir("registry-consumer");
        let cache_root = temp_dir("registry-empty-cache");
        let dep = Dependency {
            name: "nope".to_string(),
            source: DependencySource::Registry("^1.0.0".to_string()),
        };
        let err = resolve_one_dependency(&dep, &manifest_dir, &cache_root, false).unwrap_err();
        assert!(err.contains("nope"));
        assert!(err.contains("no package registry"));
    }

    #[test]
    fn fetch_git_dependency_clones_derives_a_version_and_strips_git_metadata() {
        let repo = make_git_repo(
            "git-versioned",
            Some(r#"{"name":"gitdep","version":"1.2.3","entry":"main.gx"}"#),
        );
        let cache_root = temp_dir("git-versioned-cache");
        let url = format!("file://{}", repo.display());

        let (version, resolved, dir) =
            fetch_git_dependency("gitdep", &url, None, &cache_root, false).unwrap();

        assert_eq!(version, "1.2.3");
        assert!(resolved.starts_with("git+"));
        assert!(dir.join("main.gx").exists());
        assert!(
            !dir.join(".git").exists(),
            ".git metadata must be stripped before caching"
        );
    }

    #[test]
    fn fetch_git_dependency_falls_back_to_a_pseudo_version_when_undeclared() {
        let repo = make_git_repo("git-unversioned", None);
        let cache_root = temp_dir("git-unversioned-cache");
        let url = format!("file://{}", repo.display());

        let (version, _resolved, _dir) =
            fetch_git_dependency("nover", &url, None, &cache_root, false).unwrap();

        assert!(
            version.starts_with("0.0.0+git."),
            "expected a git-sha pseudo-version, got: {}",
            version
        );
        // Must still be valid semver, since it round-trips through gx.lock
        // and (eventually) version-range matching.
        semver::Version::parse(&version).unwrap();
    }

    #[test]
    fn fetch_git_dependency_offline_with_a_warm_cache_reuses_it_without_touching_git() {
        let cache_root = temp_dir("git-offline-warm-cache");
        let dir = crate::package::cache_dir_for(&cache_root, "cached", "9.9.9");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("main.gx"), "function f() { return 1 }\n").unwrap();

        let (version, resolved, resolved_dir) = fetch_git_dependency(
            "cached",
            "https://example.invalid/nope.git",
            None,
            &cache_root,
            true,
        )
        .unwrap();

        assert_eq!(version, "9.9.9");
        assert!(resolved.contains("offline"));
        assert_eq!(resolved_dir, dir);
    }

    #[test]
    fn fetch_git_dependency_offline_with_a_cold_cache_errors_clearly() {
        let cache_root = temp_dir("git-offline-cold-cache");
        let err = fetch_git_dependency(
            "nothing-cached",
            "https://example.invalid/nope.git",
            None,
            &cache_root,
            true,
        )
        .unwrap_err();
        assert!(err.contains("--offline"));
        assert!(err.contains("nothing-cached"));
    }

    #[test]
    fn fetch_git_dependency_reports_a_clear_error_for_an_invalid_url() {
        let cache_root = temp_dir("git-bad-url-cache");
        let err = fetch_git_dependency(
            "bad",
            "file:///this/path/does/not/exist/at/all",
            None,
            &cache_root,
            false,
        )
        .unwrap_err();
        assert!(
            err.contains("bad"),
            "error should name the dependency: {}",
            err
        );
    }

    #[test]
    fn sanitize_for_temp_dir_strips_path_separators() {
        assert_eq!(sanitize_for_temp_dir("plain"), "plain");
        assert_eq!(sanitize_for_temp_dir("has/slash"), "has_slash");
        assert_eq!(sanitize_for_temp_dir("../../etc"), "______etc");
    }

    #[test]
    fn validate_git_arg_accepts_every_legitimate_transport() {
        for url in [
            "https://github.com/user/repo.git",
            "http://example.com/repo.git",
            "ssh://git@example.com/repo.git",
            "git://example.com/repo.git",
            "file:///home/user/repo",
            "git@github.com:user/repo.git",
            "v1.2.3",
            "main",
            "a1b2c3d",
        ] {
            assert!(
                validate_git_arg("url", url).is_ok(),
                "should accept: {}",
                url
            );
        }
    }

    #[test]
    fn validate_git_arg_rejects_a_leading_dash_as_argument_injection() {
        let err = validate_git_arg("url", "--upload-pack=touch pwned").unwrap_err();
        assert!(err.contains("must not start with"));
    }

    #[test]
    fn validate_git_arg_rejects_remote_helper_transports() {
        for value in ["ext::sh -c 'touch pwned'", "fd::42", "myhelper::whatever"] {
            let err = validate_git_arg("url", value).unwrap_err();
            assert!(
                err.contains("remote helper"),
                "expected a remote-helper rejection for '{}', got: {}",
                value,
                err
            );
        }
    }

    #[test]
    fn validate_git_arg_rejects_empty_values() {
        assert!(validate_git_arg("rev", "").is_err());
    }

    #[test]
    fn fetch_git_dependency_rejects_a_malicious_url_before_ever_invoking_git() {
        let cache_root = temp_dir("git-malicious-url-cache");
        let err = fetch_git_dependency(
            "evil",
            "ext::sh -c touch /tmp/gx_test_should_never_exist",
            None,
            &cache_root,
            false,
        )
        .unwrap_err();
        assert!(err.contains("evil"));
        assert!(err.contains("remote helper"));
        assert!(!std::path::Path::new("/tmp/gx_test_should_never_exist").exists());
    }
}

#[cfg(test)]
mod doc_tests {
    use super::*;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "gx_doc_test_{}_{}_{}",
            label,
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extract_doc_comment_collects_a_contiguous_comment_block_above_the_line() {
        let source = "// first line\n// second line\nfunction f() {}\n";
        let doc = extract_doc_comment(source, 3).unwrap();
        assert_eq!(doc, "first line\nsecond line");
    }

    #[test]
    fn extract_doc_comment_returns_none_when_nothing_precedes() {
        let source = "function f() {}\n";
        assert!(extract_doc_comment(source, 1).is_none());
    }

    #[test]
    fn extract_doc_comment_returns_none_when_a_blank_line_separates_it() {
        let source = "// unrelated comment\n\nfunction f() {}\n";
        assert!(extract_doc_comment(source, 3).is_none());
    }

    #[test]
    fn extract_doc_comment_stops_at_the_first_non_comment_line() {
        let source = "x = 1\n// only this line is the doc comment\nfunction f() {}\n";
        let doc = extract_doc_comment(source, 3).unwrap();
        assert_eq!(doc, "only this line is the doc comment");
    }

    #[test]
    fn doc_generates_markdown_for_a_single_file_with_functions_agents_and_tools() {
        let dir = temp_dir("single-file");
        let file = dir.join("main.gx");
        fs::write(
            &file,
            r#"// Doubles a number.
function double(x) {
  return x * 2
}

function undocumented(a, b) {
  return a + b
}

tool "lookup" {
  description: "Looks something up"
  params: {
    q: { type: "string", description: "the query", required: true }
  }
  execute(q) {
    return { ok: true }
  }
}
"#,
        )
        .unwrap();

        let out = dir.join("out.md");
        doc(file.to_str().unwrap(), Some(out.to_str().unwrap())).unwrap();
        let md = fs::read_to_string(&out).unwrap();

        assert!(md.contains("double(x)"));
        assert!(md.contains("Doubles a number."));
        assert!(md.contains("undocumented(a, b)"));
        assert!(md.contains("lookup"));
        assert!(md.contains("Looks something up"));
        assert!(md.contains("the query"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doc_covers_every_gx_file_in_a_directory() {
        let dir = temp_dir("directory");
        fs::write(dir.join("a.gx"), "function from_a() { return 1 }\n").unwrap();
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(
            dir.join("sub").join("b.gx"),
            "function from_b() { return 2 }\n",
        )
        .unwrap();

        let out = dir.join("out.md");
        doc(dir.to_str().unwrap(), Some(out.to_str().unwrap())).unwrap();
        let md = fs::read_to_string(&out).unwrap();

        assert!(md.contains("from_a"));
        assert!(md.contains("from_b"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doc_errors_clearly_when_the_target_does_not_exist() {
        let err = doc("/does/not/exist.gx", None).unwrap_err();
        assert!(err.contains("not found"));
    }
}

#[cfg(test)]
mod fmt_tests {
    use super::*;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "gx_fmt_test_{}_{}_{}",
            label,
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fmt_writes_a_reformatted_single_file_in_place() {
        let dir = temp_dir("single-file");
        let file = dir.join("a.gx");
        fs::write(&file, "agent \"x\" {\nwhen started {\nsay \"hi\"\n}\n}\n").unwrap();

        fmt(file.to_str().unwrap(), false).unwrap();

        let after = fs::read_to_string(&file).unwrap();
        assert!(after.contains("agent"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fmt_covers_every_gx_file_in_a_directory() {
        let dir = temp_dir("directory");
        fs::write(
            dir.join("a.gx"),
            "agent \"a\" {\nwhen started {\nsay \"a\"\n}\n}\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(
            dir.join("sub").join("b.gx"),
            "agent \"b\" {\nwhen started {\nsay \"b\"\n}\n}\n",
        )
        .unwrap();

        fmt(dir.to_str().unwrap(), false).unwrap();

        // Both files must have been processed without error; content
        // specifics are covered by fmt_indent_syntax/format_source's own
        // tests — this test's job is proving directory recursion works.
        assert!(fs::read_to_string(dir.join("a.gx"))
            .unwrap()
            .contains("agent"));
        assert!(fs::read_to_string(dir.join("sub").join("b.gx"))
            .unwrap()
            .contains("agent"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fmt_check_reports_unformatted_files_without_writing_them() {
        let dir = temp_dir("check-dirty");
        let file = dir.join("a.gx");
        let original = "agent \"x\" {\nwhen started{\nsay \"hi\"\n}\n}\n";
        fs::write(&file, original).unwrap();

        let err = fmt(file.to_str().unwrap(), true).unwrap_err();
        assert!(err.contains("would be reformatted"));

        // --check must never write, regardless of outcome.
        let unchanged = fs::read_to_string(&file).unwrap();
        assert_eq!(unchanged, original);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fmt_check_succeeds_when_a_file_is_already_formatted() {
        let dir = temp_dir("check-clean");
        let file = dir.join("a.gx");
        // Format it first (non-check), then --check against the result —
        // this must report "already formatted", not flag its own output.
        fs::write(&file, "agent \"x\" {\nwhen started {\nsay \"hi\"\n}\n}\n").unwrap();
        fmt(file.to_str().unwrap(), false).unwrap();

        fmt(file.to_str().unwrap(), true).unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fmt_errors_clearly_when_the_target_does_not_exist() {
        let err = fmt("/does/not/exist.gx", false).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn format_source_never_silently_drops_a_keyword() {
        // Regression test for a critical, pre-existing bug this milestone
        // uncovered via --check: token_to_str's match had a `_ =>
        // String::new()` fallback covering nearly every keyword beyond a
        // small hand-picked set (Function/"fn", Timeout, Assert, While,
        // Import, Parallel, Tool, Await, ...) — gx fmt silently deleted
        // every occurrence of any of them, corrupting real source (e.g.
        // `shared_config.timeout == 30` became `shared_config. == 30`,
        // `fn(n) { ... }` became `(n) { ... }` — a dropped `fn` that
        // turns a closure into invalid syntax). Exercises a representative
        // sample of the keywords that used to vanish.
        let source = r#"function f() {
  assert true "ok"
  x = 1
  while x < 3 {
    x += 1
  }
  y = fn(n) { return n }
  import "other.gx"
}
"#;
        let formatted = format_source(source).unwrap();
        for keyword in ["assert", "while", "function", "import"] {
            assert!(
                formatted.contains(keyword),
                "formatted output must still contain '{}', got:\n{}",
                keyword,
                formatted
            );
        }
        // Re-parsing must still succeed — a dropped keyword usually turns
        // the output into invalid syntax, not just different-looking
        // valid syntax.
        let tokens = crate::lexer::Lexer::new(&formatted).tokenize().unwrap();
        crate::parser::Parser::new(tokens).parse().unwrap();
    }

    #[test]
    fn format_source_round_trips_string_escape_sequences() {
        // Regression test: the lexer *decodes* \n/\t/\\/\" while
        // tokenizing, so a StringLit's value already holds the real
        // characters — re-emitting it verbatim (only re-escaping `"`, as
        // a previous version of this function did) wrote a literal
        // newline/tab byte into the formatted source, silently turning a
        // one-line string into a multi-line one.
        let source = "function f() {\n  x = \"a\\nb\\tc\\\\d\\\"e\"\n}\n";
        let formatted = format_source(source).unwrap();
        // The formatted source must still be exactly one function's
        // worth of lines — an unescaped \n would have split the string
        // across lines, changing the count.
        assert_eq!(formatted.lines().count(), source.lines().count());

        // And re-parsing must reconstruct the exact original string value.
        let tokens = crate::lexer::Lexer::new(&formatted).tokenize().unwrap();
        let program = crate::parser::Parser::new(tokens).parse().unwrap();
        let body = &program.functions[0].body;
        match &body[0] {
            crate::ast::Stmt::Assign {
                value: crate::ast::Expr::Str(s),
                ..
            } => assert_eq!(s, "a\nb\tc\\d\"e"),
            other => panic!("expected a string assignment, got {:?}", other),
        }
    }

    #[test]
    // Regression/guard test for a reported-but-unreproduced bug (AgentX
    // feedback, 2026-07, item 1.1): "gx fmt silently truncates the last
    // character of an identifier immediately before a closing `}`". The
    // exact repro from that report, and several structural variants of
    // it (CRLF, tabs, trailing whitespace, deeply nested blocks, `for`/
    // `while`, a bare `return`, two closing braces stacked on adjacent
    // lines), did not reproduce against this exact source tree — but
    // formatter trust is close to existential for this feature (a
    // formatter that can *ever* silently rewrite an identifier is worse
    // than no formatter), so this asserts the stronger, general property
    // directly: every `Ident` token in the source must appear, in the
    // same order and completely unchanged, in the formatted output.
    // Anything short of that is a variant of the same corruption class,
    // reproducible or not.
    fn format_source_never_alters_any_identifier_token() {
        fn ident_tokens(src: &str) -> Vec<String> {
            crate::lexer::Lexer::new(src)
                .tokenize()
                .unwrap()
                .into_iter()
                .filter_map(|t| match t.kind {
                    crate::lexer::TokenKind::Ident(s) => Some(s),
                    _ => None,
                })
                .collect()
        }

        let cases = [
            // The exact repro from the report.
            "function effective_daily_limit() {\n  limit = round(base * multiplier)\n  if limit < min_limit {\n    limit = min_limit\n  }\n  return limit\n}\n",
            // CRLF line endings.
            "function f() {\r\n  x = min_limit\r\n}\r\n",
            // Tab indentation.
            "function f() {\n\tx = min_limit\n}\n",
            // Trailing whitespace on the identifier's own line.
            "function f() {\n  x = min_limit   \n}\n",
            // Nested block, identifier immediately before a nested `}`.
            "function f() {\n  if x {\n    y = min_limit\n  }\n}\n",
            "function f() {\n  for i in range(1, 10) {\n    y = min_limit\n  }\n}\n",
            "function f() {\n  while true {\n    y = min_limit\n    break\n  }\n}\n",
            // Bare `return` immediately before `}`.
            "function f() {\n  return min_limit\n}\n",
            // Two closing braces stacked on adjacent lines (the report's
            // own function ends this way: `limit` then `}` then, one
            // level up, `return limit` then `}`).
            "function outer() {\n  function unused_helper_name() {\n    value = inner_identifier\n  }\n  return outer_identifier\n}\n",
        ];

        for source in cases {
            let formatted = format_source(source).unwrap();
            assert_eq!(
                ident_tokens(source),
                ident_tokens(&formatted),
                "gx fmt must never alter identifier tokens — source:\n{}\nformatted:\n{}",
                source,
                formatted
            );
        }
    }

    #[test]
    // Regression test: `fmt_indent_syntax` (progressive-syntax formatting)
    // used `line.trim_end()` to compute the trimmed line body, which only
    // strips *trailing* whitespace — the line's original leading
    // indentation stayed in that string and got written a second time
    // right after the freshly computed indent, so every `gx fmt` pass on
    // a progressive-syntax file grew its indentation instead of
    // normalizing it. Found while manually verifying idempotency across
    // the whole `tests/` corpus during this milestone, not something the
    // existing single-fixture idempotency test below happened to cover
    // (that fixture is brace-syntax).
    fn fmt_indent_syntax_is_idempotent() {
        let source = fs::read_to_string("tests/test_progressive_syntax.gx").unwrap();
        let once = format_source(&source).unwrap();
        let twice = format_source(&once).unwrap();
        assert_eq!(
            once, twice,
            "a second gx fmt pass must not change indentation further"
        );
        // And a third pass, for good measure — the original bug compounded further.
        let thrice = format_source(&twice).unwrap();
        assert_eq!(twice, thrice);
    }

    #[test]
    fn format_source_is_idempotent_on_a_real_project_file() {
        // The two bugs above were both discovered because a formatted
        // file, formatted a second time, produced different output — the
        // property `gx fmt --check` depends on to ever succeed right
        // after `gx fmt` itself runs. Pin it against a real, substantial
        // file rather than only small hand-written snippets.
        let source = fs::read_to_string("tests/test_v041_fixes.gx").unwrap();
        let once = format_source(&source).unwrap();
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice);
    }
}
