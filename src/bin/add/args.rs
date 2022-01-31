//! Handle `cargo add` arguments

#![allow(clippy::bool_assert_comparison)]

use cargo_edit::{
    find, get_features_from_registry, get_manifest_from_url, registry_url, workspace_members,
    Dependency,
};
use cargo_edit::{get_latest_dependency, CrateSpec};
use cargo_metadata::Package;
use clap::Parser;
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
    pub fn parse_dependencies(
        &self,
        requested_features: Option<Vec<String>>,
    ) -> Result<Vec<Dependency>> {
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
                self.parse_single_dependency(crate_spec, &workspace_members)
                    .map(|x| {
                        let x = self.populate_dependency(x);
                        let x = x.set_features(requested_features.to_owned());
                        x
                    })
            })
            .collect()
    }

    fn populate_dependency(&self, mut dependency: Dependency) -> Dependency {
        dependency = dependency
            .set_optional(self.optional())
            .set_default_features(self.default_features());
        if let Some(ref rename) = self.rename {
            dependency = dependency.set_rename(rename);
        }
        dependency
    }

    fn parse_single_dependency(
        &self,
        crate_spec: &str,
        workspace_members: &[Package],
    ) -> Result<Dependency> {
        let crate_spec = CrateSpec::resolve(crate_spec)?;
        let manifest_path = find(&self.manifest_path)?;
        let registry_url = registry_url(&manifest_path, self.registry.as_deref())?;

        let mut dependency = match &crate_spec {
            CrateSpec::PkgId {
                name: _,
                version_req: Some(_),
            } => {
                let mut dependency = crate_spec.to_dependency()?;
                // crate specifier includes a version (e.g. `docopt@0.8`)
                if let Some(ref url) = self.git {
                    let url = url.clone();
                    let version = dependency.version().unwrap().to_string();
                    return Err(ErrorKind::GitUrlWithVersion(url, version).into());
                }

                let features = get_features_from_registry(
                    &dependency.name,
                    dependency
                        .version()
                        .expect("version populated by `parse_as_version`"),
                    &registry_url,
                )?;
                dependency = dependency.set_available_features(features);

                dependency
            }
            CrateSpec::PkgId {
                name,
                version_req: None,
            } => {
                let mut dependency = crate_spec.to_dependency()?;

                if let Some(repo) = &self.git {
                    assert!(self.registry.is_none());
                    let features = get_manifest_from_url(repo)?
                        .map(|m| m.features())
                        .transpose()?
                        .unwrap_or_else(Vec::new);

                    dependency = dependency
                        .set_git(
                            repo,
                            self.branch.clone(),
                            self.tag.clone(),
                            self.rev.clone(),
                        )
                        .set_available_features(features);
                } else {
                    // Only special-case workspaces when the user doesn't provide any extra
                    // information, otherwise, trust the user.
                    if let Some(package) = workspace_members.iter().find(|p| p.name == *name) {
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
                        dependency = get_latest_dependency(
                            name,
                            false,
                            &manifest_path,
                            Some(&registry_url),
                        )?;
                        let op = "";
                        let v = format!(
                            "{op}{version}",
                            op = op,
                            // If version is unavailable `get_latest_dependency` must have
                            // returned `Err(FetchVersionError::GetVersion)`
                            version = dependency.version().unwrap_or_else(|| unreachable!())
                        );
                        dependency = dependency.set_version(&v);
                    }
                }

                dependency
            }
            CrateSpec::Path(_) => {
                let mut dependency = crate_spec.to_dependency()?;
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
    #[cfg(feature = "test-external-apis")]
    fn test_repo_as_arg_parsing() {
        let github_url = "https://github.com/killercup/cargo-edit/";
        let args_github = Args {
            crates: vec![github_url.to_owned()],
            ..Args::default()
        };
        assert_eq!(
            args_github.parse_dependencies(None).unwrap(),
            vec![Dependency::new("cargo-edit").set_git(github_url, None)]
        );

        let gitlab_url = "https://gitlab.com/Polly-lang/Polly.git";
        let args_gitlab = Args {
            crates: vec![gitlab_url.to_owned()],
            ..Args::default()
        };
        assert_eq!(
            args_gitlab.parse_dependencies(None).unwrap(),
            vec![Dependency::new("polly").set_git(gitlab_url, None)]
        );
    }

    #[test]
    fn test_path_as_arg_parsing() {
        let self_path = dunce::canonicalize(std::env::current_dir().unwrap()).unwrap();
        let args_path = Args {
            // Hacky to `display` but should generally work
            crates: vec![self_path.display().to_string()],
            ..Args::default()
        };
        assert_eq!(
            args_path.parse_dependencies(None).unwrap()[0]
                .path()
                .unwrap(),
            self_path
        );
    }

    #[test]
    fn verify_app() {
        use clap::IntoApp;
        Command::into_app().debug_assert()
    }
}
