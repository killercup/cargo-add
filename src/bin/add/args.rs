//! Handle `cargo add` arguments

#![allow(clippy::bool_assert_comparison)]

use cargo_edit::{
    get_features_from_registry, get_manifest_from_path, get_manifest_from_url, registry_url,
    workspace_members, Dependency, LocalManifest,
};
use cargo_edit::{get_latest_dependency, CrateSpec};
use cargo_metadata::Package;
use clap::Parser;
use std::path::Path;
use std::path::PathBuf;

use crate::errors::*;

#[derive(Debug, Parser)]
#[clap(bin_name = "cargo")]
pub enum Command {
    /// Add dependencies to a Cargo.toml manifest file.
    #[clap(name = "add")]
    #[clap(after_help = "\
Examples:
  $ cargo add regex
  $ cargo add regex@0.1.41 --build
  $ cargo add trycmd --dev
  $ cargo add ./crate/parser/
")]
    Add(Args),
}

#[derive(Debug, Parser)]
#[clap(about, version)]
#[clap(setting = clap::AppSettings::DeriveDisplayOrder)]
pub struct Args {
    /// Reference to a package to add as a dependency
    ///
    /// You can reference a packages by:{n}
    /// - `<name>`, like `cargo add serde` (latest version will be used){n}
    /// - `<name>@<version-req>`, like `cargo add serde@1` or `cargo add serde@=1.0.38`{n}
    /// - `<path>`, like `cargo add ./crates/parser/`
    #[clap(value_name = "DEP_ID", required = true)]
    pub crates: Vec<String>,

    /// Disable the default features
    #[clap(long)]
    no_default_features: bool,
    /// Re-enable the default features
    #[clap(long, overrides_with = "no-default-features")]
    default_features: bool,

    /// Space-separated list of features to add
    #[clap(long)]
    pub features: Option<Vec<String>>,

    /// Mark the dependency as optional
    ///
    /// The package name will be exposed as feature of your crate.
    #[clap(long, conflicts_with = "dev")]
    pub optional: bool,

    /// Mark the dependency as required
    ///
    /// The package will be removed from your features.
    #[clap(long, conflicts_with = "dev", overrides_with = "optional")]
    pub no_optional: bool,

    /// Rename the dependency
    ///
    /// Example uses:{n}
    /// - Depending on multiple versions of a crate{n}
    /// - Depend on crates with the same name from different registries
    #[clap(long, short)]
    pub rename: Option<String>,

    /// Package registry for this dependency
    #[clap(long, conflicts_with = "git")]
    pub registry: Option<String>,

    /// Add as development dependency
    ///
    /// Dev-dependencies are not used when compiling a package for building, but are used for compiling tests, examples, and benchmarks.
    ///
    /// These dependencies are not propagated to other packages which depend on this package.
    #[clap(short = 'D', long, help_heading = "SECTION", group = "section")]
    pub dev: bool,

    /// Add as build dependency
    ///
    /// Build-dependencies are the only dependencies available for use by build scripts (`build.rs`
    /// files).
    #[clap(short = 'B', long, help_heading = "SECTION", group = "section")]
    pub build: bool,

    /// Add as dependency to the given target platform.
    #[clap(
        long,
        forbid_empty_values = true,
        help_heading = "SECTION",
        group = "section"
    )]
    pub target: Option<String>,

    /// Path to `Cargo.toml`
    #[clap(long, value_name = "PATH", parse(from_os_str))]
    pub manifest_path: Option<PathBuf>,

    /// Package to modify
    #[clap(short = 'p', long = "package", value_name = "PKGID")]
    pub pkgid: Option<String>,

    /// Run without accessing the network
    #[clap(long)]
    pub offline: bool,

    /// Do not print any output in case of success.
    #[clap(long)]
    pub quiet: bool,

    /// Unstable (nightly-only) flags
    #[clap(
        short = 'Z',
        value_name = "FLAG",
        help_heading = "UNSTABLE",
        global = true,
        arg_enum
    )]
    pub unstable_features: Vec<UnstableOptions>,

    /// Git repository location
    ///
    /// Without any other information, cargo will use latest commit on the main branch.
    #[clap(long, value_name = "URI", help_heading = "UNSTABLE")]
    pub git: Option<String>,

    /// Git branch to download the crate from.
    #[clap(
        long,
        value_name = "BRANCH",
        help_heading = "UNSTABLE",
        requires = "git",
        group = "git-ref"
    )]
    pub branch: Option<String>,

    /// Git tag to download the crate from.
    #[clap(
        long,
        value_name = "TAG",
        help_heading = "UNSTABLE",
        requires = "git",
        group = "git-ref"
    )]
    pub tag: Option<String>,

    /// Git reference to download the crate from
    ///
    /// This is the catch all, handling hashes to named references in remote repositories.
    #[clap(
        long,
        value_name = "REV",
        help_heading = "UNSTABLE",
        requires = "git",
        group = "git-ref"
    )]
    pub rev: Option<String>,
}

impl Args {
    /// Get dependency section
    pub fn get_section(&self) -> Vec<String> {
        if self.dev {
            vec!["dev-dependencies".to_owned()]
        } else if self.build {
            vec!["build-dependencies".to_owned()]
        } else if let Some(ref target) = self.target {
            assert!(!target.is_empty(), "Target specification may not be empty");

            vec![
                "target".to_owned(),
                target.clone(),
                "dependencies".to_owned(),
            ]
        } else {
            vec!["dependencies".to_owned()]
        }
    }

    pub fn default_features(&self) -> Option<bool> {
        resolve_bool_arg(self.default_features, self.no_default_features)
    }

    /// Build dependencies from arguments
    pub fn parse_dependencies(&self, manifest: &LocalManifest) -> Result<Vec<Dependency>> {
        let workspace_members = workspace_members(self.manifest_path.as_deref())?;

        if self.crates.len() > 1 && self.git.is_some() {
            return Err(ErrorKind::MultipleCratesWithGitOrPathOrVers.into());
        }

        if self.crates.len() > 1 && self.rename.is_some() {
            return Err(ErrorKind::MultipleCratesWithRename.into());
        }

        if self.crates.len() > 1 && self.features.is_some() {
            return Err(ErrorKind::MultipleCratesWithFeatures.into());
        }

        self.crates
            .iter()
            .map(|crate_spec| {
                self.parse_single_dependency(manifest, crate_spec, &workspace_members)
            })
            .collect()
    }

    fn parse_single_dependency(
        &self,
        manifest: &LocalManifest,
        crate_spec: &str,
        workspace_members: &[Package],
    ) -> Result<Dependency> {
        let crate_spec = CrateSpec::resolve(crate_spec)?;
        let manifest_path = manifest.path.as_path();

        let mut dependency = match &crate_spec {
            CrateSpec::PkgId {
                name: _,
                version_req: Some(_),
            } => {
                let mut dependency = crate_spec.to_dependency()?;
                dependency = self.populate_dependency(dependency);
                // crate specifier includes a version (e.g. `docopt@0.8`)
                if let Some(ref url) = self.git {
                    let url = url.clone();
                    let version = dependency.version().unwrap().to_string();
                    return Err(ErrorKind::GitUrlWithVersion(url, version).into());
                }

                dependency
            }
            CrateSpec::PkgId {
                name,
                version_req: None,
            } => {
                let mut dependency = crate_spec.to_dependency()?;
                dependency = self.populate_dependency(dependency);

                if let Some(repo) = &self.git {
                    assert!(self.registry.is_none());
                    dependency = dependency.set_git(
                        repo,
                        self.branch.clone(),
                        self.tag.clone(),
                        self.rev.clone(),
                    );
                } else if let Some(version) =
                    manifest.get_dep_version(&self.get_section(), dependency.toml_key())
                {
                    dependency = dependency.set_version(&version);
                } else if let Some(package) = workspace_members.iter().find(|p| p.name == *name) {
                    // Only special-case workspaces when the user doesn't provide any extra
                    // information, otherwise, trust the user.
                    dependency = dependency.set_path(
                        package
                            .manifest_path
                            .parent()
                            .expect("at least parent dir")
                            .as_std_path()
                            .to_owned(),
                    );
                    // dev-dependencies do not need the version populated
                    if !self.dev {
                        let op = "";
                        let v = format!("{op}{version}", op = op, version = package.version);
                        dependency = dependency.set_version(&v);
                    }
                } else {
                    let registry_url = registry_url(manifest_path, self.registry.as_deref())?;
                    let latest =
                        get_latest_dependency(name, false, manifest_path, Some(&registry_url))?;
                    let op = "";
                    let v = format!(
                        "{op}{version}",
                        op = op,
                        // If version is unavailable `get_latest_dependency` must have
                        // returned `Err(FetchVersionError::GetVersion)`
                        version = latest.version().unwrap_or_else(|| unreachable!())
                    );
                    dependency = dependency
                        .set_version(&v)
                        .set_available_features(latest.available_features);
                }

                dependency
            }
            CrateSpec::Path(_) => {
                let mut dependency = crate_spec.to_dependency()?;
                dependency = self.populate_dependency(dependency);

                // dev-dependencies do not need the version populated
                if !self.dev {
                    let dep_path = dependency.path().map(ToOwned::to_owned);
                    if let Some(dep_path) = dep_path {
                        if let Some(package) = workspace_members.iter().find(|p| {
                            p.manifest_path.parent().map(|p| p.as_std_path())
                                == Some(dep_path.as_path())
                        }) {
                            let op = "";
                            let v = format!("{op}{version}", op = op, version = package.version);

                            dependency = dependency.set_version(&v);
                        }
                    }
                }
                dependency
            }
        };

        if let Some(registry) = &self.registry {
            dependency = dependency.set_registry(registry);
        }
        dependency = self.populate_available_features(dependency, manifest_path)?;

        Ok(dependency)
    }

    fn populate_dependency(&self, mut dependency: Dependency) -> Dependency {
        let requested_features: Option<Vec<_>> = self.features.as_ref().map(|v| {
            v.iter()
                .flat_map(|s| s.split(' '))
                .flat_map(|s| s.split(','))
                .filter(|s| !s.is_empty())
                .map(|f| f.to_owned())
                .collect()
        });

        dependency = dependency
            .set_optional(self.optional())
            .set_default_features(self.default_features())
            .set_features(requested_features);

        if let Some(ref rename) = self.rename {
            dependency = dependency.set_rename(rename);
        }

        dependency
    }

    /// Lookup available features
    pub fn populate_available_features(
        &self,
        dependency: Dependency,
        manifest_path: &Path,
    ) -> Result<Dependency> {
        if !dependency.available_features.is_empty() {
            return Ok(dependency);
        }

        let available_features = if let Some(path) = dependency.path() {
            let manifest = get_manifest_from_path(path)?;
            manifest.features()?
        } else if let Some(repo) = dependency.git() {
            get_manifest_from_url(repo)?
                .map(|m| m.features())
                .transpose()?
                .unwrap_or_else(Vec::new)
        } else if let Some(version) = dependency.version() {
            let registry_url = registry_url(manifest_path, self.registry.as_deref())?;
            get_features_from_registry(&dependency.name, version, &registry_url)?
        } else {
            vec![]
        };

        let dependency = dependency.set_available_features(available_features);
        Ok(dependency)
    }
}

impl Args {
    pub fn optional(&self) -> Option<bool> {
        resolve_bool_arg(self.optional, self.no_optional)
    }
}

#[cfg(test)]
impl Default for Args {
    fn default() -> Args {
        Args {
            crates: vec!["demo".to_owned()],
            rename: None,
            dev: false,
            build: false,
            git: None,
            branch: None,
            tag: None,
            rev: None,
            target: None,
            optional: false,
            no_optional: false,
            manifest_path: None,
            pkgid: None,
            features: None,
            no_default_features: false,
            default_features: false,
            quiet: false,
            offline: true,
            registry: None,
            unstable_features: vec![],
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ArgEnum)]
pub enum UnstableOptions {
    Git,
}

fn resolve_bool_arg(yes: bool, no: bool) -> Option<bool> {
    match (yes, no) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
        (_, _) => unreachable!("clap should make this impossible"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_app() {
        use clap::IntoApp;
        Command::into_app().debug_assert()
    }
}
