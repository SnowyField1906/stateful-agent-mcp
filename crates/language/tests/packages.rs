//! `Packages`: manifests mark projects, and the origin of a package is always well formed.

use stateful_agent_language::{Origin, Package};

use super::support::{name, syntaxes};
use stateful_agent_testkit::TempDir;

#[test]
fn manifests_are_bare_file_names() {
    for syntax in syntaxes() {
        for manifest in syntax.manifests() {
            assert!(
                !manifest.is_empty() && !manifest.contains(['/', '\\']),
                "{}: `{manifest}` is not a file name",
                name(syntax)
            );
        }
    }
}

#[test]
fn a_folder_is_a_workspace_only_when_its_manifest_says_so() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let project = TempDir::new("language-workspace");
        assert!(
            !syntax.declares_workspace(&project.root),
            "{language}: a folder without a manifest is a workspace"
        );
        for manifest in syntax.manifests() {
            project.write(manifest, "");
            assert!(
                !syntax.declares_workspace(&project.root),
                "{language}: an empty `{manifest}` declares a workspace"
            );
        }
    }
}

#[test]
fn a_relative_package_is_the_projects_and_a_dependency_is_named_after_its_root() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let project = TempDir::new("language-origin");
        let file = format!("src/main.{}", syntax.language().extensions[0]);
        project.write(&file, "");
        for manifest in syntax.manifests() {
            project.write(manifest, "");
        }
        let from = project.root.join(&file);
        let origin = |root: &str, relative: bool| {
            syntax.package_origin(&Package {
                root,
                relative,
                from: &from,
                project: &project.root,
            })
        };

        assert_eq!(
            origin("sibling", true),
            Origin::Project,
            "{language}: a relative import is not the project's code"
        );
        let root = "no_such_package_anywhere";
        if let Origin::Dependency(package) = origin(root, false) {
            assert_eq!(package, root, "{language}: the dependency has another name");
        }
    }
}

#[test]
fn entrypoints_are_existing_files_of_the_project() {
    for syntax in syntaxes() {
        let language = name(syntax);
        let project = TempDir::new("language-entrypoints");
        assert!(
            syntax.entrypoints(&project.root).is_empty(),
            "{language}: an empty folder has entrypoints"
        );
        project.write(&format!("src/main.{}", syntax.language().extensions[0]), "");
        for manifest in syntax.manifests() {
            project.write(manifest, "");
        }
        for entry in syntax.entrypoints(&project.root) {
            assert!(
                entry.is_file() && entry.starts_with(&project.root),
                "{language}: {} is not a file of the project",
                entry.display()
            );
        }
    }
}
