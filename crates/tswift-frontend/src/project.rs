//! A bounded `Package.swift` **manifest reader**, not a Swift Package Manager.
//!
//! SwiftPM manifests are executable Swift — the "real" way to read one is to
//! compile and *run* it, which this crate deliberately never does (no
//! arbitrary manifest code execution). Instead [`parse_manifest`] pattern-
//! matches the manifest's parse AST for the one shape every manifest shares:
//! a top-level `let package = Package(name: …, targets: […])` literal. This
//! covers the common case (name + `.executableTarget`/`.target` entries with
//! optional `path:`) and explicitly tolerates-and-ignores every construct it
//! doesn't model (`platforms:`, `products:`, `dependencies:`, target
//! `resources:`/`swiftSettings:`/…): those are only ever load-bearing for a
//! *build graph* we don't have, never for *which files make up a target*,
//! which is all [`load_program`] needs.
//!
//! [`load_program`] then turns a manifest (or, lacking one, a flat directory)
//! into the ordered [`SourceFile`] list for one target — the same program-
//! input contract [`crate::Analysis::analyze_program`] already consumes.

use crate::SourceFile;

/// The kind of a target entry recognized in a `targets:` array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetKind {
    /// `.executableTarget(...)` — produces a `main.swift`-rooted program.
    Executable,
    /// `.target(...)` — a library target; not directly runnable.
    Library,
    /// A recognized-but-unsupported target constructor (`.testTarget`,
    /// `.binaryTarget`, `.systemLibrary`, `.plugin`, …), kept by its call
    /// name so a *user request* naming this exact target still gets a clear
    /// diagnostic instead of silently vanishing.
    Other(String),
}

/// One target entry read from a manifest's `targets:` array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTarget {
    pub name: String,
    /// Explicit `path:` argument, if written. `None` means the SwiftPM
    /// convention default (`Sources/<name>/`).
    pub path: Option<String>,
    pub kind: TargetKind,
    /// Explicit `sources:` file list, if written — each entry a path
    /// relative to the target's directory (`path`, or the convention
    /// default). When present, this is the *exact* source set (no directory
    /// recursion); when absent, every `.swift` file under the target
    /// directory is used, per SwiftPM's own default.
    pub sources: Vec<String>,
    /// Explicit `exclude:` path list, if written — each entry a literal
    /// path (file or directory) relative to the target's directory to skip
    /// during directory-recursion source discovery. No glob support (v1).
    /// Ignored when `sources:` is also present (SwiftPM applies `exclude:`
    /// only to the directory-scan default, not to an explicit list).
    pub exclude: Vec<String>,
    /// Names of other targets this target depends on, from a literal
    /// `dependencies: ["Name", ...]` array of string literals (SwiftPM also
    /// accepts `.target(name:)`/`.product(name:package:)` entries and
    /// package-external products; those are silently dropped from this list
    /// rather than erroring, since `tswift test`'s only use for this field is
    /// concatenating a **local** dependency target's sources into a test
    /// unit — see [`crate::project::load_test_program`]).
    ///
    /// This is only the *direct* edge list; `load_test_program` walks it
    /// transitively (BFS, cycle-guarded via `transitive_dependencies`), so a
    /// chain like `testTarget -> Core -> Base` pulls in `Base`'s sources too,
    /// not just `Core`'s.
    pub dependencies: Vec<String>,
}

/// The subset of a `Package.swift` manifest this reader extracts: the
/// package name and its target list (in source order). Everything else the
/// manifest may declare (`platforms:`, `products:`, `dependencies:`, …) is
/// read and discarded — see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageManifest {
    pub name: String,
    pub targets: Vec<PackageTarget>,
}

impl PackageManifest {
    /// The target named `name`, if the manifest declares one.
    pub fn target(&self, name: &str) -> Option<&PackageTarget> {
        self.targets.iter().find(|t| t.name == name)
    }

    /// The first `.executableTarget` in source order, if any.
    pub fn first_executable(&self) -> Option<&PackageTarget> {
        self.targets
            .iter()
            .find(|t| t.kind == TargetKind::Executable)
    }
}

/// Why [`parse_manifest`] could not extract a [`PackageManifest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// `source` does not parse as Swift at all.
    SyntaxError(String),
    /// No top-level `let package = Package(…)` literal call was found. This
    /// is the one load-bearing construct the reader requires: a manifest
    /// that builds its `Package` value any other way (a helper function, a
    /// conditional, a computed property) cannot be pattern-matched without
    /// executing it, which this reader will not do.
    NotAManifest,
    /// The `Package(…)` call has no literal `name:` argument (or its value
    /// isn't a plain string literal). The package name is load-bearing for
    /// diagnostics and target-source-set derivation, so this is fatal rather
    /// than tolerated.
    MissingName,
    /// A `targets:` array element could not be modeled at all — not a
    /// literal `.xTarget(...)` call (e.g. a variable, a conditional
    /// expression, or a constructor this reader doesn't recognize), or
    /// missing a literal `name:` argument. Unlike `platforms:`/`products:`/
    /// unrecognized *argument labels* within a target call (silently
    /// tolerated, per the module docs), an entire target entry that can't be
    /// read is load-bearing: silently dropping it could select the wrong
    /// executable target, so it is always a hard error rather than a silent
    /// skip.
    MalformedTarget(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::SyntaxError(msg) => write!(f, "Package.swift: {msg}"),
            ManifestError::NotAManifest => write!(
                f,
                "Package.swift: no `let package = Package(name:targets:)` literal found \
                 (dynamically-constructed manifests are not supported: tswift reads the \
                 manifest's syntax, it does not execute it)"
            ),
            ManifestError::MissingName => write!(
                f,
                "Package.swift: `Package(...)` is missing a literal `name:` argument"
            ),
            ManifestError::MalformedTarget(reason) => {
                write!(f, "Package.swift: `targets:` entry {reason}")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

/// Parse a `Package.swift` manifest **source** into its [`PackageManifest`]
/// shape, by pattern-matching the parse AST. Never executes the manifest.
pub fn parse_manifest(source: &str) -> Result<PackageManifest, ManifestError> {
    let analysis = crate::Analysis::analyze(source, "Package.swift")
        .map_err(|e| ManifestError::SyntaxError(e.to_string()))?;
    if let Some(err) = analysis.diagnostics().into_iter().find(|d| d.is_error()) {
        return Err(ManifestError::SyntaxError(err.message));
    }

    let call = find_package_call(analysis.root()).ok_or(ManifestError::NotAManifest)?;

    let mut name = None;
    let mut targets = Vec::new();
    for arg in call.children().skip(1) {
        match arg.arg_label().as_deref() {
            Some("name") => name = string_literal_value(&arg),
            Some("targets") => targets = parse_targets(&arg)?,
            // `platforms:`, `products:`, `dependencies:`, `swiftLanguageModes:`,
            // trailing positional args, … — read and discarded (see module docs).
            _ => {}
        }
    }

    Ok(PackageManifest {
        name: name.ok_or(ManifestError::MissingName)?,
        targets,
    })
}

/// Find the top-level `let package = Package(…)` (or `var package = …`)
/// binding's initializer call.
fn find_package_call(root: crate::Node<'_>) -> Option<crate::Node<'_>> {
    root.children()
        .filter(|c| {
            matches!(
                c.kind(),
                crate::NodeKind::LetDecl | crate::NodeKind::VarDecl
            ) && c.decl_name().as_deref() == Some("package")
        })
        .find_map(|decl| {
            decl.children().find(|c| {
                c.kind() == crate::NodeKind::CallExpr
                    && c.first_child().and_then(|callee| callee.text()).as_deref()
                        == Some("Package")
            })
        })
}

/// Extract target entries from a `targets:` argument's value. Only a literal
/// `ArrayLiteral` of `.executableTarget(...)`/`.target(...)`/other-known-
/// constructor calls is understood; any array element with a shape this
/// reader can't statically read (a variable, a conditional expression, an
/// unrecognized constructor, a constructor missing a literal `name:`) is a
/// hard [`ManifestError::MalformedTarget`] — never silently dropped, since a
/// partial target list could select the wrong executable target (see that
/// variant's doc).
fn parse_targets(value: &crate::Node<'_>) -> Result<Vec<PackageTarget>, ManifestError> {
    if value.kind() != crate::NodeKind::ArrayLiteral {
        return Err(ManifestError::MalformedTarget(format!(
            "is not a literal array (found `{:?}`)",
            value.kind()
        )));
    }
    value
        .children()
        .map(|elt| parse_target_entry(&elt))
        .collect()
}

fn parse_target_entry(elt: &crate::Node<'_>) -> Result<PackageTarget, ManifestError> {
    let malformed = |reason: String| ManifestError::MalformedTarget(reason);

    if elt.kind() != crate::NodeKind::CallExpr {
        return Err(malformed(format!(
            "is not a `.xTarget(...)` call (found `{:?}`)",
            elt.kind()
        )));
    }
    let callee = elt
        .first_child()
        .ok_or_else(|| malformed("is a call with no callee".to_string()))?;
    if callee.kind() != crate::NodeKind::MemberExpr {
        return Err(malformed(
            "is a call whose target constructor is not a `.member` form".to_string(),
        ));
    }
    let ctor = callee
        .text()
        .ok_or_else(|| malformed("has an unreadable target constructor name".to_string()))?;
    let kind = match ctor.as_str() {
        "executableTarget" => TargetKind::Executable,
        "target" => TargetKind::Library,
        "testTarget" | "binaryTarget" | "systemLibrary" | "plugin" => {
            TargetKind::Other(ctor.clone())
        }
        other => {
            return Err(malformed(format!(
                "uses an unrecognized constructor `.{other}`"
            )));
        }
    };

    let mut name = None;
    let mut path = None;
    let mut sources = Vec::new();
    let mut exclude = Vec::new();
    let mut dependencies = Vec::new();
    for arg in elt.children().skip(1) {
        match arg.arg_label().as_deref() {
            Some("name") => name = string_literal_value(&arg),
            Some("path") => path = string_literal_value(&arg),
            Some("sources") => {
                sources = string_array_value(&arg).ok_or_else(|| {
                    malformed(format!(
                        "`.{ctor}`'s `sources:` is not a literal array of strings"
                    ))
                })?;
            }
            Some("exclude") => {
                exclude = string_array_value(&arg).ok_or_else(|| {
                    malformed(format!(
                        "`.{ctor}`'s `exclude:` is not a literal array of strings"
                    ))
                })?;
            }
            Some("dependencies") => dependencies = dependency_names_value(&arg),
            _ => {}
        }
    }

    let name =
        name.ok_or_else(|| malformed(format!("`.{ctor}` is missing a literal `name:` argument")))?;

    Ok(PackageTarget {
        name,
        path,
        kind,
        sources,
        exclude,
        dependencies,
    })
}

/// Extract target-local dependency names from a `dependencies:` array.
/// Recognizes bare string literals (`"Core"`) and `.target(name: "Core")`
/// entries; anything else (`.product(name:package:)`, a variable, string
/// interpolation) names a target this reader cannot resolve locally and is
/// silently dropped — see [`PackageTarget::dependencies`]'s doc for why that's
/// safe here (it only ever widens a test unit's source set, never narrows the
/// requested target itself).
fn dependency_names_value(value: &crate::Node<'_>) -> Vec<String> {
    if value.kind() != crate::NodeKind::ArrayLiteral {
        return Vec::new();
    }
    value
        .children()
        .filter_map(|elt| match elt.kind() {
            crate::NodeKind::StringLiteral => string_literal_value(&elt),
            crate::NodeKind::CallExpr => {
                let callee = elt.first_child()?;
                if callee.kind() != crate::NodeKind::MemberExpr
                    || callee.text().as_deref() != Some("target")
                {
                    return None;
                }
                elt.children()
                    .skip(1)
                    .find(|a| a.arg_label().as_deref() == Some("name"))
                    .and_then(|a| string_literal_value(&a))
            }
            _ => None,
        })
        .collect()
}

/// The unescaped values of a literal `ArrayLiteral` of `StringLiteral`
/// elements. `None` if `value` isn't an array literal or contains any
/// element that isn't a plain string literal.
fn string_array_value(value: &crate::Node<'_>) -> Option<Vec<String>> {
    if value.kind() != crate::NodeKind::ArrayLiteral {
        return None;
    }
    value
        .children()
        .map(|elt| string_literal_value(&elt))
        .collect()
}

/// The unescaped value of a plain (non-interpolated) `StringLiteral` node.
/// Manifest string arguments (names/paths) are always simple literals in
/// practice; this deliberately does not support `\(…)` interpolation.
fn string_literal_value(node: &crate::Node<'_>) -> Option<String> {
    if node.kind() != crate::NodeKind::StringLiteral {
        return None;
    }
    let raw = node.text()?;
    let body = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => break,
        }
    }
    Some(out)
}

/// Why [`load_program`] could not derive a program's [`SourceFile`] list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectError {
    /// The manifest itself failed to parse (see [`ManifestError`]).
    Manifest(ManifestError),
    /// No `target` was requested and the manifest declares no
    /// `.executableTarget`.
    NoExecutableTarget,
    /// The requested target name isn't declared by the manifest.
    TargetNotFound(String),
    /// The requested target exists but isn't a runnable kind (e.g.
    /// `.testTarget`) — load-bearing: the user asked for this exact target.
    UnsupportedTargetKind { name: String, ctor: String },
    /// The target's source directory (by convention or explicit `path:`)
    /// contributed no `.swift` files.
    NoSourceFiles { target: String, dir: String },
    /// The target's explicit `sources:` list names a path that doesn't
    /// exist among the project's files.
    MissingSourceFile { target: String, path: String },
    /// Flat-directory fallback (no `Package.swift`) found no `.swift` files.
    NoSwiftFiles,
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::Manifest(e) => write!(f, "{e}"),
            ProjectError::NoExecutableTarget => write!(
                f,
                "Package.swift declares no `.executableTarget`; pass an explicit target name"
            ),
            ProjectError::TargetNotFound(name) => {
                write!(f, "Package.swift declares no target named `{name}`")
            }
            ProjectError::UnsupportedTargetKind { name, ctor } => {
                write!(f, "target `{name}` is a `.{ctor}`, which tswift cannot run")
            }
            ProjectError::NoSourceFiles { target, dir } => write!(
                f,
                "target `{target}`'s source directory `{dir}` contains no `.swift` files"
            ),
            ProjectError::MissingSourceFile { target, path } => write!(
                f,
                "target `{target}`'s `sources:` names `{path}`, which does not exist"
            ),
            ProjectError::NoSwiftFiles => write!(f, "no `.swift` files found"),
        }
    }
}

impl std::error::Error for ProjectError {}

/// Derive the ordered [`SourceFile`] program input for one executable target
/// out of a project's full file listing (every file under the project root,
/// `path` relative to that root — e.g. `"Package.swift"`,
/// `"Sources/App/main.swift"`).
///
/// * If `files` contains a `"Package.swift"` (matched by exact relative
///   path, i.e. at the project root), it is parsed via [`parse_manifest`] and
///   drives target selection: `target` names the target to build, or (when
///   `None`) the manifest's first `.executableTarget` is used. That target's
///   source files are every `.swift` file under its directory (`path:` if
///   given, else the `Sources/<name>/` convention), recursively.
/// * Otherwise this is flat-directory mode: every `.swift` file in `files`
///   is returned, sorted by path — the pre-existing `tswift run <dir>`
///   convention.
///
/// Either way the result is sorted by path for determinism, matching the
/// `main.swift`-is-entry convention `Analysis::analyze_program` enforces.
pub fn load_program(
    files: &[SourceFile],
    target: Option<&str>,
) -> Result<Vec<SourceFile>, ProjectError> {
    match files.iter().find(|f| f.path == "Package.swift") {
        Some(manifest_file) => load_manifest_program(files, &manifest_file.source, target),
        None => {
            let mut out: Vec<SourceFile> = files
                .iter()
                .filter(|f| f.path.ends_with(".swift"))
                .cloned()
                .collect();
            if out.is_empty() {
                return Err(ProjectError::NoSwiftFiles);
            }
            out.sort_by(|a, b| a.path.cmp(&b.path));
            Ok(out)
        }
    }
}

fn load_manifest_program(
    files: &[SourceFile],
    manifest_source: &str,
    target: Option<&str>,
) -> Result<Vec<SourceFile>, ProjectError> {
    let manifest = parse_manifest(manifest_source).map_err(ProjectError::Manifest)?;

    let chosen = match target {
        Some(name) => manifest
            .target(name)
            .ok_or_else(|| ProjectError::TargetNotFound(name.to_string()))?,
        None => manifest
            .first_executable()
            .ok_or(ProjectError::NoExecutableTarget)?,
    };
    if let TargetKind::Other(ctor) = &chosen.kind {
        return Err(ProjectError::UnsupportedTargetKind {
            name: chosen.name.clone(),
            ctor: ctor.clone(),
        });
    }

    let mut out = target_source_files(files, chosen)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// The ordered (unsorted) `.swift` files that make up `target`'s own source
/// set — `sources:`-listed files if given, else every `.swift` file under its
/// directory (`path:`, or the `Sources/<name>/` convention), minus
/// `exclude:`. Shared by [`load_manifest_program`] (one runnable target) and
/// [`load_test_program`] (a test target plus its local dependencies).
fn target_source_files(
    files: &[SourceFile],
    target: &PackageTarget,
) -> Result<Vec<SourceFile>, ProjectError> {
    // SwiftPM's own convention default differs by kind: library/executable
    // targets live under `Sources/<name>/`, test targets under `Tests/<name>/`.
    let convention_root = match &target.kind {
        TargetKind::Other(c) if c == "testTarget" => "Tests",
        _ => "Sources",
    };
    let dir = target
        .path
        .clone()
        .unwrap_or_else(|| format!("{convention_root}/{}", target.name));
    let prefix = format!("{}/", dir.trim_end_matches('/'));

    let out: Vec<SourceFile> = if !target.sources.is_empty() {
        // Explicit `sources:` list: the exact source set, each entry a path
        // relative to `dir`. No directory recursion, no `exclude:` (SwiftPM
        // applies `exclude:` only to the directory-scan default).
        target
            .sources
            .iter()
            .map(|rel| {
                let full = format!("{prefix}{}", rel.trim_start_matches('/'));
                files
                    .iter()
                    .find(|f| f.path == full)
                    .cloned()
                    .ok_or_else(|| ProjectError::MissingSourceFile {
                        target: target.name.clone(),
                        path: full.clone(),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        files
            .iter()
            .filter(|f| f.path.starts_with(&prefix) && f.path.ends_with(".swift"))
            .filter(|f| !is_excluded(&f.path, &prefix, &target.exclude))
            .cloned()
            .collect()
    };
    if out.is_empty() {
        return Err(ProjectError::NoSourceFiles {
            target: target.name.clone(),
            dir,
        });
    }
    Ok(out)
}

/// One test target's derived program input: its own sources plus every
/// locally-resolvable `dependencies:` target's sources, concatenated into one
/// flat compilation unit (plan §2.2 — tswift has no link step, so a test
/// target and the library it tests are analyzed together, same as multi-file
/// `run`). Files are de-duplicated by path (a dependency and the test target
/// itself never overlap in practice, but a dependency named twice must not
/// duplicate its sources) and sorted for determinism.
#[derive(Debug, Clone)]
pub struct TestProgram {
    /// The `.testTarget`'s name.
    pub target: String,
    pub files: Vec<SourceFile>,
}

/// Derive one [`TestProgram`] per selected `.testTarget` out of a project's
/// full file listing (see [`load_program`] for the `files` shape).
///
/// * `target = Some(name)` selects exactly that target, which must be a
///   `.testTarget`, or [`ProjectError::UnsupportedTargetKind`] (a *library* or
///   *executable* target named this way is a load-bearing user mistake, not a
///   silent fallback — same policy as [`load_program`]'s `UnsupportedTargetKind`,
///   mirrored the other direction).
/// * `target = None` selects every `.testTarget` the manifest declares, in
///   manifest order; declaring none is [`ProjectError::NoExecutableTarget`]
///   (reused: "no runnable target found" reads the same for either target
///   kind this loader wants).
pub fn load_test_program(
    files: &[SourceFile],
    target: Option<&str>,
) -> Result<Vec<TestProgram>, ProjectError> {
    let manifest_file = files
        .iter()
        .find(|f| f.path == "Package.swift")
        .ok_or(ProjectError::NoSwiftFiles)?;
    let manifest = parse_manifest(&manifest_file.source).map_err(ProjectError::Manifest)?;

    let is_test_target =
        |t: &&PackageTarget| matches!(&t.kind, TargetKind::Other(c) if c == "testTarget");
    let chosen: Vec<&PackageTarget> = match target {
        Some(name) => {
            let t = manifest
                .target(name)
                .ok_or_else(|| ProjectError::TargetNotFound(name.to_string()))?;
            if !is_test_target(&t) {
                let ctor = match &t.kind {
                    TargetKind::Executable => "executableTarget".to_string(),
                    TargetKind::Library => "target".to_string(),
                    TargetKind::Other(c) => c.clone(),
                };
                return Err(ProjectError::UnsupportedTargetKind {
                    name: t.name.clone(),
                    ctor,
                });
            }
            vec![t]
        }
        None => {
            let ts: Vec<&PackageTarget> = manifest.targets.iter().filter(is_test_target).collect();
            if ts.is_empty() {
                return Err(ProjectError::NoExecutableTarget);
            }
            ts
        }
    };

    chosen
        .into_iter()
        .map(|t| {
            let mut unit = target_source_files(files, t)?;
            for dep in transitive_dependencies(&manifest, t) {
                unit.extend(target_source_files(files, dep)?);
            }
            unit.sort_by(|a, b| a.path.cmp(&b.path));
            unit.dedup_by(|a, b| a.path == b.path);
            Ok(TestProgram {
                target: t.name.clone(),
                files: unit,
            })
        })
        .collect()
}

/// Every target `root` transitively depends on (BFS over `dependencies:`,
/// `root` itself excluded), in discovery order. A name with no matching
/// manifest target is silently skipped — same policy as
/// [`PackageTarget::dependencies`]'s doc (a package-external product or an
/// unresolvable name is dropped, not an error, since this loader only wants
/// *local* target sources). A cycle (direct or indirect) is guarded by a
/// visited-set: each target is queued at most once, so `A -> B -> A` still
/// terminates and includes both exactly once.
fn transitive_dependencies<'a>(
    manifest: &'a PackageManifest,
    root: &PackageTarget,
) -> Vec<&'a PackageTarget> {
    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    visited.insert(root.name.as_str());
    let mut queue: std::collections::VecDeque<&str> =
        root.dependencies.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    while let Some(name) = queue.pop_front() {
        if !visited.insert(name) {
            continue;
        }
        if let Some(dep) = manifest.target(name) {
            out.push(dep);
            queue.extend(dep.dependencies.iter().map(String::as_str));
        }
    }
    out
}

/// Whether `path` (a project-root-relative file path already known to start
/// with `prefix`, the target directory) matches an `exclude:` entry.
/// `exclude` entries are literal paths relative to the target directory
/// (`prefix`) — no glob support (v1). An entry matches a file exactly, or a
/// directory entry matches every file under it (a `/`-bounded prefix match,
/// so `"Extra"` excludes `"Extra/X.swift"` but not `"ExtraFile.swift"`).
fn is_excluded(path: &str, prefix: &str, exclude: &[String]) -> bool {
    let rel = path.strip_prefix(prefix).unwrap_or(path);
    exclude.iter().any(|entry| {
        let entry = entry.trim_matches('/');
        rel == entry || rel.starts_with(&format!("{entry}/"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\"),\n    ]\n)\n";

    #[test]
    fn parses_minimal_manifest() {
        let m = parse_manifest(MINIMAL).unwrap();
        assert_eq!(m.name, "Foo");
        assert_eq!(m.targets.len(), 1);
        assert_eq!(m.targets[0].name, "App");
        assert_eq!(m.targets[0].kind, TargetKind::Executable);
        assert_eq!(m.targets[0].path, None);
    }

    #[test]
    fn parses_explicit_path() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\", path: \"Src/App\"),\n    ]\n)\n";
        let m = parse_manifest(src).unwrap();
        assert_eq!(m.targets[0].path.as_deref(), Some("Src/App"));
    }

    #[test]
    fn parses_multiple_targets_mixed_kinds() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\"),\n        .target(name: \"Core\"),\n        .testTarget(name: \"AppTests\"),\n    ]\n)\n";
        let m = parse_manifest(src).unwrap();
        assert_eq!(m.targets.len(), 3);
        assert_eq!(m.targets[0].kind, TargetKind::Executable);
        assert_eq!(m.targets[1].kind, TargetKind::Library);
        assert_eq!(
            m.targets[2].kind,
            TargetKind::Other("testTarget".to_string())
        );
        assert_eq!(m.first_executable().unwrap().name, "App");
    }

    #[test]
    fn ignores_platforms_products_and_dependencies() {
        let src = "let package = Package(\n    name: \"Foo\",\n    platforms: [.iOS(.v13)],\n    products: [.library(name: \"Foo\", targets: [\"Core\"])],\n    dependencies: [.package(url: \"https://example.com/x\", from: \"1.0.0\")],\n    targets: [\n        .executableTarget(name: \"App\", dependencies: [\"Core\"]),\n        .target(name: \"Core\"),\n    ]\n)\n";
        let m = parse_manifest(src).unwrap();
        assert_eq!(m.name, "Foo");
        assert_eq!(m.targets.len(), 2);
    }

    /// A manifest that doesn't build `Package` via a literal `let package =
    /// Package(...)` call is a load-bearing unsupported construct: it can't
    /// be pattern-matched without executing the manifest, so it's a clear
    /// error rather than a silent empty result.
    #[test]
    fn dynamic_manifest_is_a_clear_diagnostic() {
        let src = "func makePackage() -> Package {\n    Package(name: \"Foo\", targets: [])\n}\nlet package = makePackage()\n";
        let err = parse_manifest(src).unwrap_err();
        assert_eq!(err, ManifestError::NotAManifest);
    }

    #[test]
    fn missing_name_is_a_clear_diagnostic() {
        let src = "let package = Package(targets: [.executableTarget(name: \"App\")])\n";
        let err = parse_manifest(src).unwrap_err();
        assert_eq!(err, ManifestError::MissingName);
    }

    fn sf(path: &str, source: &str) -> SourceFile {
        SourceFile::new(path, source)
    }

    #[test]
    fn loader_derives_convention_sources_for_executable_target() {
        let files = [
            sf("Package.swift", MINIMAL),
            sf("Sources/App/main.swift", "print(1)\n"),
            sf("Sources/App/Helper.swift", "func h() {}\n"),
            sf("README.md", "# Foo\n"),
        ];
        let out = load_program(&files, None).unwrap();
        assert_eq!(
            out.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["Sources/App/Helper.swift", "Sources/App/main.swift"]
        );
    }

    #[test]
    fn loader_honours_explicit_path() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\", path: \"Src/App\"),\n    ]\n)\n";
        let files = [
            sf("Package.swift", src),
            sf("Src/App/main.swift", "print(1)\n"),
            sf("Sources/App/main.swift", "print(2)\n"), // convention dir, should be ignored
        ];
        let out = load_program(&files, None).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, "Src/App/main.swift");
    }

    #[test]
    fn loader_selects_named_target() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\"),\n        .executableTarget(name: \"Tool\"),\n    ]\n)\n";
        let files = [
            sf("Package.swift", src),
            sf("Sources/App/main.swift", "print(1)\n"),
            sf("Sources/Tool/main.swift", "print(2)\n"),
        ];
        let out = load_program(&files, Some("Tool")).unwrap();
        assert_eq!(out[0].path, "Sources/Tool/main.swift");
    }

    #[test]
    fn loader_reports_missing_target() {
        let files = [
            sf("Package.swift", MINIMAL),
            sf("Sources/App/main.swift", "print(1)\n"),
        ];
        let err = load_program(&files, Some("Nope")).unwrap_err();
        assert_eq!(err, ProjectError::TargetNotFound("Nope".to_string()));
    }

    #[test]
    fn loader_reports_no_executable_target() {
        let src = "let package = Package(name: \"Foo\", targets: [.target(name: \"Core\")])\n";
        let files = [
            sf("Package.swift", src),
            sf("Sources/Core/lib.swift", "func f(){}\n"),
        ];
        let err = load_program(&files, None).unwrap_err();
        assert_eq!(err, ProjectError::NoExecutableTarget);
    }

    #[test]
    fn loader_reports_unsupported_target_kind_when_requested() {
        let src =
            "let package = Package(name: \"Foo\", targets: [.testTarget(name: \"AppTests\")])\n";
        let files = [
            sf("Package.swift", src),
            sf("Tests/AppTests/T.swift", "func t(){}\n"),
        ];
        let err = load_program(&files, Some("AppTests")).unwrap_err();
        assert_eq!(
            err,
            ProjectError::UnsupportedTargetKind {
                name: "AppTests".to_string(),
                ctor: "testTarget".to_string()
            }
        );
    }

    #[test]
    fn falls_back_to_flat_directory_mode_without_manifest() {
        let files = [
            sf("main.swift", "print(1)\n"),
            sf("Helper.swift", "func h() {}\n"),
            sf("notes.txt", "hi\n"),
        ];
        let out = load_program(&files, None).unwrap();
        assert_eq!(
            out.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["Helper.swift", "main.swift"]
        );
    }

    /// A `targets:` array element that isn't a literal `.xTarget(...)` call
    /// (here a bare identifier, as a conditional/dynamic entry would also
    /// desugar to something non-call-shaped) must be a hard error, never a
    /// silent drop — dropping it could silently select the wrong executable
    /// target.
    #[test]
    fn malformed_target_entry_is_a_clear_diagnostic_not_silently_dropped() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        someTargetVar,\n    ]\n)\n";
        let err = parse_manifest(src).unwrap_err();
        assert!(matches!(err, ManifestError::MalformedTarget(_)), "{err:?}");
    }

    /// An unrecognized target constructor name is malformed, not silently
    /// skipped.
    #[test]
    fn unrecognized_target_constructor_is_a_clear_diagnostic() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .customTarget(name: \"X\"),\n    ]\n)\n";
        let err = parse_manifest(src).unwrap_err();
        assert!(matches!(err, ManifestError::MalformedTarget(_)), "{err:?}");
    }

    /// A target entry missing a literal `name:` is malformed, not silently
    /// skipped.
    #[test]
    fn target_entry_missing_name_is_a_clear_diagnostic() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(path: \"Foo\"),\n    ]\n)\n";
        let err = parse_manifest(src).unwrap_err();
        assert!(matches!(err, ManifestError::MalformedTarget(_)), "{err:?}");
    }

    #[test]
    fn loader_honours_explicit_sources_list() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\", sources: [\"main.swift\"]),\n    ]\n)\n";
        let files = [
            sf("Package.swift", src),
            sf("Sources/App/main.swift", "print(1)\n"),
            // Present on disk but NOT named in `sources:` — must be excluded.
            sf("Sources/App/Helper.swift", "func h() {}\n"),
        ];
        let out = load_program(&files, None).unwrap();
        assert_eq!(
            out.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["Sources/App/main.swift"]
        );
    }

    #[test]
    fn loader_reports_missing_explicit_source_file() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\", sources: [\"main.swift\", \"Missing.swift\"]),\n    ]\n)\n";
        let files = [
            sf("Package.swift", src),
            sf("Sources/App/main.swift", "print(1)\n"),
        ];
        let err = load_program(&files, None).unwrap_err();
        assert_eq!(
            err,
            ProjectError::MissingSourceFile {
                target: "App".to_string(),
                path: "Sources/App/Missing.swift".to_string()
            }
        );
    }

    #[test]
    fn parses_target_dependencies() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\", dependencies: [\"Core\", .target(name: \"Utils\")]),\n    ]\n)\n";
        let m = parse_manifest(src).unwrap();
        assert_eq!(
            m.targets[0].dependencies,
            vec!["Core".to_string(), "Utils".to_string()]
        );
    }

    fn test_project() -> Vec<SourceFile> {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\", dependencies: [\"Core\"]),\n        .target(name: \"Core\"),\n        .testTarget(name: \"AppTests\", dependencies: [\"Core\"]),\n    ]\n)\n";
        vec![
            sf("Package.swift", src),
            sf("Sources/App/main.swift", "print(1)\n"),
            sf("Sources/Core/lib.swift", "func h() -> Int { 1 }\n"),
            sf(
                "Tests/AppTests/T.swift",
                "@Test func t() { #expect(h() == 1) }\n",
            ),
        ]
    }

    #[test]
    fn load_test_program_selects_all_test_targets_by_default() {
        let files = test_project();
        let programs = load_test_program(&files, None).unwrap();
        assert_eq!(programs.len(), 1);
        assert_eq!(programs[0].target, "AppTests");
    }

    #[test]
    fn load_test_program_concatenates_dependency_sources() {
        let files = test_project();
        let programs = load_test_program(&files, None).unwrap();
        let paths: Vec<&str> = programs[0].files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["Sources/Core/lib.swift", "Tests/AppTests/T.swift"]
        );
    }

    #[test]
    fn load_test_program_selects_named_target() {
        let files = test_project();
        let programs = load_test_program(&files, Some("AppTests")).unwrap();
        assert_eq!(programs.len(), 1);
        assert_eq!(programs[0].target, "AppTests");
    }

    #[test]
    fn load_test_program_rejects_non_test_target_by_name() {
        let files = test_project();
        let err = load_test_program(&files, Some("App")).unwrap_err();
        assert_eq!(
            err,
            ProjectError::UnsupportedTargetKind {
                name: "App".to_string(),
                ctor: "executableTarget".to_string()
            }
        );
    }

    #[test]
    fn load_test_program_reports_no_test_targets() {
        let src =
            "let package = Package(name: \"Foo\", targets: [.executableTarget(name: \"App\")])\n";
        let files = [
            sf("Package.swift", src),
            sf("Sources/App/main.swift", "print(1)\n"),
        ];
        let err = load_test_program(&files, None).unwrap_err();
        assert_eq!(err, ProjectError::NoExecutableTarget);
    }

    /// A test target's dependency chain is resolved transitively: `AppTests`
    /// depends on `Core`, `Core` depends on `Base` (not on `AppTests`
    /// directly) — `Base`'s sources must still land in the test unit (BFS/DFS
    /// over the dependency graph, not a single hop).
    #[test]
    fn load_test_program_resolves_transitive_dependencies() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .target(name: \"Base\"),\n        .target(name: \"Core\", dependencies: [\"Base\"]),\n        .testTarget(name: \"AppTests\", dependencies: [\"Core\"]),\n    ]\n)\n";
        let files = [
            sf("Package.swift", src),
            sf("Sources/Base/base.swift", "func base() -> Int { 1 }\n"),
            sf("Sources/Core/core.swift", "func core() -> Int { base() }\n"),
            sf(
                "Tests/AppTests/T.swift",
                "@Test func t() { #expect(core() == 1) }\n",
            ),
        ];
        let programs = load_test_program(&files, None).unwrap();
        assert_eq!(programs.len(), 1);
        let paths: Vec<&str> = programs[0].files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "Sources/Base/base.swift",
                "Sources/Core/core.swift",
                "Tests/AppTests/T.swift"
            ]
        );
    }

    /// A dependency cycle (`A -> B -> A`) must not hang or blow the stack —
    /// each target's sources are still included exactly once.
    #[test]
    fn load_test_program_tolerates_dependency_cycle() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .target(name: \"A\", dependencies: [\"B\"]),\n        .target(name: \"B\", dependencies: [\"A\"]),\n        .testTarget(name: \"AppTests\", dependencies: [\"A\"]),\n    ]\n)\n";
        let files = [
            sf("Package.swift", src),
            sf("Sources/A/a.swift", "func a() {}\n"),
            sf("Sources/B/b.swift", "func b() {}\n"),
            sf("Tests/AppTests/T.swift", "@Test func t() { a() }\n"),
        ];
        let programs = load_test_program(&files, None).unwrap();
        let paths: Vec<&str> = programs[0].files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "Sources/A/a.swift",
                "Sources/B/b.swift",
                "Tests/AppTests/T.swift"
            ]
        );
    }

    #[test]
    fn loader_honours_exclude_list() {
        let src = "let package = Package(\n    name: \"Foo\",\n    targets: [\n        .executableTarget(name: \"App\", exclude: [\"Extra\", \"Skip.swift\"]),\n    ]\n)\n";
        let files = [
            sf("Package.swift", src),
            sf("Sources/App/main.swift", "print(1)\n"),
            sf("Sources/App/Skip.swift", "func skip() {}\n"),
            sf("Sources/App/Extra/Nested.swift", "func n() {}\n"),
            // Must NOT be excluded by a prefix-match on "Extra" as a
            // substring — exclude entries are path-segment-bounded.
            sf("Sources/App/ExtraFile.swift", "func e() {}\n"),
        ];
        let out = load_program(&files, None).unwrap();
        assert_eq!(
            out.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["Sources/App/ExtraFile.swift", "Sources/App/main.swift"]
        );
    }
}
