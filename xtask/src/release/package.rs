//! `xtask release package` and `xtask release oci-push`: packages every
//! release artifact of both Products from one set of built (and, in release
//! runs, signed) binaries:
//! - GitHub Release `.tar.xz` archives, split debug symbols and `CycloneDX` SBOMs;
//! - shared components packaged once (`goodfetch-proving-ground`,
//!   `goodfetch-sandbox`, `goodfetch-filter-broker`);
//! - npm packages (`@goodfetch/collie-client`, `@goodfetch/<product>` and its
//!   six `@goodfetch/<product>-<platform>-<arch>` packages, with exact
//!   `optionalDependencies` and zero install scripts);
//! - Python `py3-none` platform wheels and the pure `goodfetch-collie-client`
//!   wheel;
//! - first-party Plugin multi-platform OCI Image Layouts (ADRs 0024, 0032,
//!   0033, 0090, 0094 and 0095; issue 141).

mod oci_layout;
mod zip;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result, ensure};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use super::{ProductVersion, assets};
use crate::output;
use crate::products::Product;
use crate::release::package::zip::ZipWriter;

/// One Node platform package's target mapping: Linux uses the static musl
/// build so one package runs on both glibc and Alpine musl (ADR 0090).
struct PlatformSpec {
    target: &'static str,
    npm_os: &'static str,
    npm_cpu: &'static str,
    npm_suffix: &'static str,
    wheel_tag: &'static str,
    windows: bool,
}

const PLATFORMS: [PlatformSpec; 6] = [
    PlatformSpec {
        target: "aarch64-apple-darwin",
        npm_os: "darwin",
        npm_cpu: "arm64",
        npm_suffix: "darwin-arm64",
        wheel_tag: "macosx_11_0_arm64",
        windows: false,
    },
    PlatformSpec {
        target: "x86_64-apple-darwin",
        npm_os: "darwin",
        npm_cpu: "x64",
        npm_suffix: "darwin-x64",
        wheel_tag: "macosx_10_15_x86_64",
        windows: false,
    },
    PlatformSpec {
        target: "aarch64-unknown-linux-musl",
        npm_os: "linux",
        npm_cpu: "arm64",
        npm_suffix: "linux-arm64",
        wheel_tag: "manylinux_2_17_aarch64.musllinux_1_2_aarch64",
        windows: false,
    },
    PlatformSpec {
        target: "x86_64-unknown-linux-musl",
        npm_os: "linux",
        npm_cpu: "x64",
        npm_suffix: "linux-x64",
        wheel_tag: "manylinux_2_17_x86_64.musllinux_1_2_x86_64",
        windows: false,
    },
    PlatformSpec {
        target: "aarch64-pc-windows-msvc",
        npm_os: "win32",
        npm_cpu: "arm64",
        npm_suffix: "win32-arm64",
        wheel_tag: "win_arm64",
        windows: true,
    },
    PlatformSpec {
        target: "x86_64-pc-windows-msvc",
        npm_os: "win32",
        npm_cpu: "x64",
        npm_suffix: "win32-x64",
        wheel_tag: "win_amd64",
        windows: true,
    },
];

#[derive(clap::Args)]
pub(crate) struct PackageArguments {
    /// Product version for every generated manifest, wheel, SBOM and OCI layout.
    #[arg(long)]
    version: ProductVersion,
    /// Folder holding `<target>/<binary>` and split debug files.
    #[arg(long)]
    binaries: PathBuf,
    /// Destination folder (`assets/`, `npm/`, `wheels/`, `oci/`).
    #[arg(long)]
    out: PathBuf,
    /// Targets to package; defaults to all eight release targets.
    #[arg(long, value_delimiter = ',')]
    targets: Vec<String>,
}

#[derive(clap::Args)]
pub(crate) struct OciPushArguments {
    /// OCI Image Layout folder (or `.oci.tar` archive) containing `oci-layout`,
    /// `index.json` and `blobs/`.
    #[arg(long)]
    layout: PathBuf,
    /// OCI registry base URL (`https://ghcr.io` or local `registry:2`).
    #[arg(long)]
    registry: String,
    /// Target repository path (`goodfetch/goodfetch-plugin-<name>`).
    #[arg(long)]
    repository: String,
    /// Tag to apply to the pushed multi-platform index.
    #[arg(long)]
    tag: String,
    /// Optional path where the pushed top-level manifest digest (`sha256:...`)
    /// is written for `actions/attest-build-provenance`.
    #[arg(long)]
    digest_out: Option<PathBuf>,
}

pub(super) fn run_package(root: &Path, arguments: &PackageArguments) -> Result<ExitCode> {
    let semver = arguments.version.semver();
    let pep440 = arguments.version.pep440();
    let binaries = root.join(&arguments.binaries);
    let out = root.join(&arguments.out);
    let targets: Vec<String> = if arguments.targets.is_empty() {
        assets::TARGETS.iter().map(|t| (*t).to_owned()).collect()
    } else {
        arguments.targets.clone()
    };

    let assets_dir = out.join("assets");
    let npm_dir = out.join("npm");
    let wheels_dir = out.join("wheels");
    let plugins_dir = out.join("oci").join("plugins");
    let charts_dir = out.join("oci").join("charts");
    for dir in [
        &assets_dir,
        &npm_dir,
        &wheels_dir,
        &plugins_dir,
        &charts_dir,
    ] {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    package_archives(&binaries, &targets, &assets_dir)?;
    package_sboms(root, &semver, &assets_dir)?;
    package_catalog(root, &assets_dir)?;
    package_seccomp_profile(root, &assets_dir)?;
    package_npm(root, &semver, &binaries, &targets, &npm_dir)?;
    package_wheels(root, &pep440, &binaries, &targets, &wheels_dir)?;
    package_plugins(root, &semver, &binaries, &plugins_dir)?;
    package_helm_charts(root, &semver, &charts_dir)?;

    output::line(&arguments.out.display().to_string());
    Ok(ExitCode::SUCCESS)
}

fn package_seccomp_profile(root: &Path, assets_dir: &Path) -> Result<()> {
    let source = root.join("packaging/oci/seccomp-profile.json");
    if !source.is_file() {
        return Ok(());
    }
    let bytes = std::fs::read(&source).with_context(|| format!("reading {}", source.display()))?;
    let _: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("validating JSON in {}", source.display()))?;
    let dest = assets_dir.join("goodfetch-seccomp.json");
    std::fs::write(&dest, bytes).with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

fn package_helm_charts(root: &Path, semver: &str, charts_dir: &Path) -> Result<()> {
    let chart_src = root.join("packaging/helm/goodfetch");
    if !chart_src.join("Chart.yaml").is_file() {
        return Ok(());
    }
    for product in Product::ALL {
        let key = product.key();
        oci_layout::build_helm_chart_layout(&chart_src, key, semver, &charts_dir.join(key))?;
    }
    Ok(())
}

fn package_catalog(root: &Path, assets_dir: &Path) -> Result<()> {
    let source = root.join(crate::tree::PUBLISHED_CATALOG);
    let bytes = std::fs::read(&source).with_context(|| {
        format!(
            "reading {} (run `xtask codegen` first)",
            crate::tree::PUBLISHED_CATALOG
        )
    })?;
    goodfetch_protocol::published_catalog::PublishedCatalog::read(&bytes)
        .with_context(|| format!("validating {}", source.display()))?;
    let dest = assets_dir.join(assets::CATALOG_FILE);
    std::fs::write(&dest, &bytes).with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

pub(super) fn run_oci_push(
    root: &Path,
    arguments: &OciPushArguments,
    token: Option<&str>,
) -> Result<ExitCode> {
    let layout = root.join(&arguments.layout);
    let digest = oci_layout::push_layout(
        &layout,
        &arguments.registry,
        &arguments.repository,
        &arguments.tag,
        token,
    )?;
    if let Some(digest_out) = &arguments.digest_out {
        let dest = root.join(digest_out);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&dest, format!("{digest}\n"))
            .with_context(|| format!("writing {}", dest.display()))?;
    }
    output::line(&format!(
        "{}/{}:{}",
        arguments.registry.trim_end_matches('/'),
        arguments.repository,
        arguments.tag
    ));
    Ok(ExitCode::SUCCESS)
}

fn exe_name(base: &str, target: &str) -> String {
    if target.contains("-windows-") {
        format!("{base}.exe")
    } else {
        base.to_owned()
    }
}

fn debug_file_name(executable: &str, target: &str) -> String {
    if target.contains("-windows-") {
        format!("{}.pdb", executable.replace('-', "_"))
    } else if target.contains("-apple-") {
        format!("{executable}.dSYM")
    } else {
        format!("{executable}.dwp")
    }
}

fn package_archives(binaries: &Path, targets: &[String], assets_dir: &Path) -> Result<()> {
    for target in targets {
        let target_dir = binaries.join(target);
        for product in Product::ALL {
            let executable = product.names().executable;
            let bin_name = exe_name(executable, target);
            let bin_path = target_dir.join(&bin_name);
            ensure!(
                bin_path.is_file(),
                "missing built executable {}",
                bin_path.display()
            );
            write_tar_xz(
                &bin_path,
                &assets_dir.join(assets::archive(product, target)),
            )?;

            let dbg_name = debug_file_name(executable, target);
            let dbg_path = target_dir.join(&dbg_name);
            ensure!(
                dbg_path.exists(),
                "missing split debug symbols {}",
                dbg_path.display()
            );
            write_tar_xz(
                &dbg_path,
                &assets_dir.join(format!("{executable}-{target}-debug.tar.xz")),
            )?;
        }

        // Shared helpers built once per target, not once per Product (ADR 0094).
        for shared in [
            "goodfetch-proving-ground",
            "goodfetch-sandbox",
            "goodfetch-filter-broker",
        ] {
            let bin_name = exe_name(shared, target);
            let bin_path = target_dir.join(&bin_name);
            if bin_path.is_file() {
                write_tar_xz(
                    &bin_path,
                    &assets_dir.join(format!("{shared}-{target}.tar.xz")),
                )?;
            }
        }
    }
    Ok(())
}

fn write_tar_xz(source: &Path, archive: &Path) -> Result<()> {
    let parent = source
        .parent()
        .with_context(|| format!("{} has no parent folder", source.display()))?;
    let name = source
        .file_name()
        .with_context(|| format!("{} has no file name", source.display()))?;
    let status = Command::new("tar")
        .arg("-cJf")
        .arg(archive)
        .arg("-C")
        .arg(parent)
        .arg(name)
        .status()
        .with_context(|| format!("running `tar -cJf {}`", archive.display()))?;
    ensure!(
        status.success(),
        "`tar -cJf {}` exited with {status}",
        archive.display()
    );
    Ok(())
}

fn package_sboms(root: &Path, semver: &str, assets_dir: &Path) -> Result<()> {
    let graph = read_lock_graph(root)?;
    for product in Product::ALL {
        let executable = product.names().executable;
        let components = product_components(&graph, executable);
        let sbom = json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.5",
            "version": 1,
            "metadata": {
                "component": {
                    "type": "application",
                    "name": executable,
                    "version": semver,
                    "purl": format!("pkg:cargo/{executable}@{semver}"),
                }
            },
            "components": components,
        });
        let path = assets_dir.join(format!("{executable}.cdx.json"));
        std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&sbom)?))
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

struct LockPackage {
    name: String,
    version: String,
    dependencies: Vec<String>,
}

fn read_lock_graph(root: &Path) -> Result<Vec<LockPackage>> {
    let lock_path = root.join("Cargo.lock");
    let Ok(text) = std::fs::read_to_string(&lock_path) else {
        return Ok(Vec::new());
    };
    let doc: toml::Table =
        toml::from_str(&text).with_context(|| format!("parsing {}", lock_path.display()))?;
    let mut out = Vec::new();
    if let Some(packages) = doc.get("package").and_then(toml::Value::as_array) {
        for pkg in packages {
            let Some(name) = pkg.get("name").and_then(toml::Value::as_str) else {
                continue;
            };
            let Some(version) = pkg.get("version").and_then(toml::Value::as_str) else {
                continue;
            };
            let dependencies = pkg
                .get("dependencies")
                .and_then(toml::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            out.push(LockPackage {
                name: name.to_owned(),
                version: version.to_owned(),
                dependencies,
            });
        }
    }
    Ok(out)
}

/// Returns the transitive `Cargo.lock` dependency closure of `root_crate` (never
/// leaking `xtask` or Foxhound-only crates into Collie's SBOM). Falls back to
/// every lock entry only when `root_crate` is absent from a synthetic fixture lock.
fn product_components(packages: &[LockPackage], root_crate: &str) -> Vec<Value> {
    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (idx, pkg) in packages.iter().enumerate() {
        by_name.entry(pkg.name.as_str()).or_default().push(idx);
    }
    let Some(roots) = by_name.get(root_crate) else {
        return packages
            .iter()
            .map(|pkg| {
                json!({
                    "type": "library",
                    "name": pkg.name,
                    "version": pkg.version,
                    "purl": format!("pkg:cargo/{}@{}", pkg.name, pkg.version),
                })
            })
            .collect();
    };
    let mut visited = std::collections::BTreeSet::new();
    let mut queue = std::collections::VecDeque::new();
    for &idx in roots {
        if visited.insert(idx) {
            queue.push_back(idx);
        }
    }
    while let Some(idx) = queue.pop_front() {
        for dep in &packages[idx].dependencies {
            let mut parts = dep.split_whitespace();
            let Some(dep_name) = parts.next() else {
                continue;
            };
            let dep_ver = parts.next();
            let Some(candidates) = by_name.get(dep_name) else {
                continue;
            };
            for &cand in candidates {
                if dep_ver.is_none_or(|ver| packages[cand].version == ver) && visited.insert(cand) {
                    queue.push_back(cand);
                }
            }
        }
    }
    let mut selected: Vec<(&str, &str)> = visited
        .into_iter()
        .map(|idx| (packages[idx].name.as_str(), packages[idx].version.as_str()))
        .collect();
    selected.sort_unstable();
    selected
        .into_iter()
        .map(|(name, version)| {
            json!({
                "type": "library",
                "name": name,
                "version": version,
                "purl": format!("pkg:cargo/{name}@{version}"),
            })
        })
        .collect()
}

fn package_npm(
    root: &Path,
    semver: &str,
    binaries: &Path,
    targets: &[String],
    npm_dir: &Path,
) -> Result<()> {
    let client_dir = npm_dir.join("collie-client");
    stage_npm_package(root, "collie-client", semver, None, &client_dir)?;
    pack_npm_tarball(
        &client_dir,
        &npm_dir.join(format!("goodfetch-collie-client-{semver}.tgz")),
    )?;

    for product in Product::ALL {
        let key = product.key();
        let executable = product.names().executable;
        let mut optional = Map::new();
        for spec in PLATFORMS {
            let pkg_name = format!("@goodfetch/{key}-{}", spec.npm_suffix);
            optional.insert(pkg_name.clone(), Value::String(semver.to_owned()));
            if !targets.iter().any(|target| target == spec.target) {
                continue;
            }
            let bin_name = exe_name(executable, spec.target);
            let source_bin = binaries.join(spec.target).join(&bin_name);
            ensure!(
                source_bin.is_file(),
                "missing executable {} for npm package {pkg_name}",
                source_bin.display()
            );
            let pkg_dir = npm_dir.join(format!("{key}-{}", spec.npm_suffix));
            let bin_dir = pkg_dir.join("bin");
            std::fs::create_dir_all(&bin_dir)?;
            let dest_bin = bin_dir.join(&bin_name);
            std::fs::copy(&source_bin, &dest_bin)?;
            set_executable_mode(&dest_bin, spec.windows)?;

            let manifest = json!({
                "name": pkg_name,
                "version": semver,
                "description": format!("{} executable for {}-{}", product.names().connection, spec.npm_os, spec.npm_cpu),
                "license": "MIT OR Apache-2.0",
                "repository": {
                    "type": "git",
                    "url": "https://github.com/goodfetch/goodfetch.git"
                },
                "os": [spec.npm_os],
                "cpu": [spec.npm_cpu],
                "files": ["bin"],
                "preferUnplugged": true,
            });
            std::fs::write(
                pkg_dir.join("package.json"),
                format!("{}\n", serde_json::to_string_pretty(&manifest)?),
            )?;
            pack_npm_tarball(
                &pkg_dir,
                &npm_dir.join(format!("goodfetch-{key}-{}-{semver}.tgz", spec.npm_suffix)),
            )?;
        }
        let root_dir = npm_dir.join(key);
        stage_npm_package(root, key, semver, Some(optional), &root_dir)?;
        pack_npm_tarball(
            &root_dir,
            &npm_dir.join(format!("goodfetch-{key}-{semver}.tgz")),
        )?;
    }
    Ok(())
}

/// Packs a staged npm package directory into a standard npm `.tgz` archive
/// rooted at `package/` so `xtask release size-manifest` measures real packed
/// npm tarballs and `npm install <tarball>` works offline.
fn pack_npm_tarball(pkg_dir: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::create(dest)
        .with_context(|| format!("creating npm tarball {}", dest.display()))?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder
        .append_dir_all("package", pkg_dir)
        .with_context(|| format!("packing npm directory {}", pkg_dir.display()))?;
    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn stage_npm_package(
    root: &Path,
    package_folder: &str,
    semver: &str,
    optional_deps: Option<Map<String, Value>>,
    dest: &Path,
) -> Result<()> {
    let source_dir = root.join("sdks/typescript").join(package_folder);
    std::fs::create_dir_all(dest)?;

    let raw = std::fs::read_to_string(source_dir.join("package.json"))
        .with_context(|| format!("reading {}/package.json", source_dir.display()))?;
    let mut manifest: Map<String, Value> = serde_json::from_str(&raw)?;
    manifest.remove("private");
    manifest.remove("scripts");
    manifest.remove("devDependencies");
    manifest.insert("version".to_owned(), Value::String(semver.to_owned()));

    if let Some(deps) = manifest
        .get_mut("dependencies")
        .and_then(Value::as_object_mut)
        && deps.contains_key("@goodfetch/collie-client")
    {
        deps.insert(
            "@goodfetch/collie-client".to_owned(),
            Value::String(semver.to_owned()),
        );
    }
    if let Some(optional) = optional_deps {
        manifest.insert("optionalDependencies".to_owned(), Value::Object(optional));
    }

    let tsconfig = source_dir.join("tsconfig.json");
    let src_dir = source_dir.join("src");
    if tsconfig.is_file() && src_dir.is_dir() {
        let dist_dir = dest.join("dist");
        let output = Command::new("pnpm")
            .current_dir(root)
            .args([
                "exec",
                "tsc",
                "-p",
                &tsconfig.to_string_lossy(),
                "--noEmit",
                "false",
                "--emitDeclarationOnly",
                "false",
                "--rewriteRelativeImportExtensions",
                "true",
                "--rootDir",
                &src_dir.to_string_lossy(),
                "--outDir",
                &dist_dir.to_string_lossy(),
            ])
            .output()
            .with_context(|| format!("compiling TypeScript package {package_folder}"))?;
        ensure!(
            output.status.success(),
            "tsc failed for {package_folder}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        rewrite_ts_paths(&mut manifest);
        manifest.insert("files".to_owned(), json!(["dist", "README.md"]));
    }

    let readme = source_dir.join("README.md");
    if readme.is_file() {
        std::fs::copy(&readme, dest.join("README.md"))?;
    }
    std::fs::write(
        dest.join("package.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    Ok(())
}

fn rewrite_ts_paths(manifest: &mut Map<String, Value>) {
    if let Some(exports) = manifest.get_mut("exports").and_then(Value::as_object_mut) {
        for value in exports.values_mut() {
            if let Some(src_path) = value.as_str()
                && let Some(stem) = src_path
                    .strip_prefix("./src/")
                    .and_then(|rest| rest.strip_suffix(".ts"))
            {
                *value = json!({
                    "types": format!("./dist/{stem}.d.ts"),
                    "default": format!("./dist/{stem}.js"),
                });
            }
        }
    }
    if let Some(bins) = manifest.get_mut("bin").and_then(Value::as_object_mut) {
        for value in bins.values_mut() {
            if let Some(src_path) = value.as_str()
                && let Some(stem) = src_path
                    .strip_prefix("./src/")
                    .and_then(|rest| rest.strip_suffix(".ts"))
            {
                *value = Value::String(format!("./dist/{stem}.js"));
            }
        }
    }
}

fn package_wheels(
    root: &Path,
    pep440: &str,
    binaries: &Path,
    targets: &[String],
    wheels_dir: &Path,
) -> Result<()> {
    write_wheel(
        root,
        "collie-client",
        "goodfetch_collie_client",
        pep440,
        "any",
        true,
        None,
        wheels_dir,
    )?;

    for product in Product::ALL {
        let key = product.key();
        let dist = product.names().import;
        let executable = product.names().executable;
        for spec in PLATFORMS {
            if !targets.iter().any(|target| target == spec.target) {
                continue;
            }
            let bin_name = exe_name(executable, spec.target);
            let bin_path = binaries.join(spec.target).join(&bin_name);
            ensure!(
                bin_path.is_file(),
                "missing executable {} for wheel {dist}",
                bin_path.display()
            );
            write_wheel(
                root,
                key,
                dist,
                pep440,
                spec.wheel_tag,
                false,
                Some((&bin_name, &bin_path)),
                wheels_dir,
            )?;
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "wheel assembly takes the package coordinates and optional platform binary directly"
)]
fn write_wheel(
    root: &Path,
    folder: &str,
    dist_name: &str,
    pep440: &str,
    platform_tag: &str,
    pure: bool,
    binary: Option<(&str, &Path)>,
    wheels_dir: &Path,
) -> Result<()> {
    let pkg_dir = root.join("sdks/python").join(folder);
    let pyproject_text = std::fs::read_to_string(pkg_dir.join("pyproject.toml"))
        .with_context(|| format!("reading {}/pyproject.toml", pkg_dir.display()))?;
    let pyproject: toml::Table = toml::from_str(&pyproject_text)?;
    let project = pyproject
        .get("project")
        .and_then(toml::Value::as_table)
        .with_context(|| format!("{folder}/pyproject.toml has no [project]"))?;
    let project_name = project
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or(dist_name);
    let requires_python = project
        .get("requires-python")
        .and_then(toml::Value::as_str)
        .unwrap_or(">=3.11");

    let mut files: BTreeMap<String, (Vec<u8>, u32)> = BTreeMap::new();
    let src_root = pkg_dir.join("src");
    if src_root.is_dir() {
        collect_python_sources(&src_root, &src_root, &mut files)?;
    }
    if let Some((bin_name, bin_path)) = binary {
        let bytes =
            std::fs::read(bin_path).with_context(|| format!("reading {}", bin_path.display()))?;
        files.insert(format!("{dist_name}/bin/{bin_name}"), (bytes, 0o755));
    }

    let dist_info = format!("{dist_name}-{pep440}.dist-info");
    let metadata = build_python_metadata(project_name, pep440, requires_python, project);
    let wheel_meta = build_wheel_metadata(platform_tag, pure);
    files.insert(
        format!("{dist_info}/METADATA"),
        (metadata.into_bytes(), 0o644),
    );
    files.insert(
        format!("{dist_info}/WHEEL"),
        (wheel_meta.into_bytes(), 0o644),
    );

    let record_path = format!("{dist_info}/RECORD");
    let mut record = String::new();
    let mut writer = ZipWriter::new();
    for (rel, (bytes, mode)) in &files {
        writer.add_file(rel, bytes, *mode)?;
        let digest = data_encoding::BASE64URL_NOPAD.encode(&Sha256::digest(bytes));
        let _ = writeln!(record, "{rel},sha256={digest},{}", bytes.len());
    }
    let _ = writeln!(record, "{record_path},,");
    writer.add_file(&record_path, record.as_bytes(), 0o644)?;

    let wheel_file = wheels_dir.join(format!("{dist_name}-{pep440}-py3-none-{platform_tag}.whl"));
    std::fs::write(&wheel_file, writer.finish()?)
        .with_context(|| format!("writing {}", wheel_file.display()))?;
    Ok(())
}

fn build_python_metadata(
    name: &str,
    pep440: &str,
    requires_python: &str,
    project: &toml::Table,
) -> String {
    let mut out = format!(
        "Metadata-Version: 2.3\nName: {name}\nVersion: {pep440}\nRequires-Python: {requires_python}\n"
    );
    if let Some(description) = project.get("description").and_then(toml::Value::as_str) {
        let _ = writeln!(out, "Summary: {description}");
    }
    if let Some(deps) = project.get("dependencies").and_then(toml::Value::as_array) {
        for dep in deps.iter().filter_map(toml::Value::as_str) {
            let rewritten = if dep.starts_with("goodfetch-collie-client==") {
                format!("goodfetch-collie-client=={pep440}")
            } else {
                dep.to_owned()
            };
            let _ = writeln!(out, "Requires-Dist: {rewritten}");
        }
    }
    out
}

fn build_wheel_metadata(platform_tag: &str, pure: bool) -> String {
    let purelib = if pure { "true" } else { "false" };
    let mut out =
        format!("Wheel-Version: 1.0\nGenerator: goodfetch-xtask\nRoot-Is-Purelib: {purelib}\n");
    for tag in platform_tag.split('.') {
        let _ = writeln!(out, "Tag: py3-none-{tag}");
    }
    out
}

fn collect_python_sources(
    base: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, (Vec<u8>, u32)>,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        if name == "__pycache__" {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_python_sources(base, &path, out)?;
        } else if entry.file_type()?.is_file() {
            let rel = path
                .strip_prefix(base)?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path)?;
            out.insert(rel, (bytes, 0o644));
        }
    }
    Ok(())
}

fn package_plugins(root: &Path, semver: &str, binaries: &Path, plugins_dir: &Path) -> Result<()> {
    for plugin in assets::first_party_plugins(root)? {
        let layout_dir = plugins_dir.join(&plugin);
        oci_layout::write_plugin_layout(&plugin, semver, binaries, &layout_dir)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable_mode(path: &Path, windows: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    if !windows {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_executable_mode(_path: &Path, _windows: bool) -> Result<()> {
    Ok(())
}

pub(super) fn update_helm_chart_yaml(text: &str, semver: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for line in text.lines() {
        if line.starts_with("version:") {
            out.push_str("version: ");
            out.push_str(semver);
        } else if line.starts_with("appVersion:") {
            out.push_str("appVersion: \"");
            out.push_str(semver);
            out.push('"');
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}
