use agent_mcp::ServerConfig;
use agent_mcp::langtool::tools::RunActionParams;

use crate::common::TempProject;

#[cfg(unix)]
#[test]
fn run_action_runs_configured_commands() {
    let tmp = TempProject::new("actions");
    let state = tmp.open(ServerConfig::default());
    tmp.write(
        ".AGENTS/config.toml",
        "[rust]\nbuild = \"echo building\"\ntest = \"echo testing {args}\"\n",
    );
    let run = |language: &str, action: &str, args: Option<&str>| {
        state.langtool_run_action(RunActionParams {
            language: language.into(),
            action: action.into(),
            args: args.map(String::from),
        })
    };

    let out = run("rust", "build", None).unwrap();
    assert!(
        out.contains("building") && out.contains("exit 0"),
        "unexpected output: {out}"
    );
    assert!(
        run("rust", "test", Some("-q"))
            .unwrap()
            .contains("testing -q")
    );

    // Missing or unexpected `args` still run the command and append a hint.
    let no_args = run("rust", "test", None).unwrap();
    assert!(
        no_args.contains("testing") && no_args.contains("--help"),
        "unexpected output: {no_args}"
    );
    let extra_args = run("rust", "build", Some("--release")).unwrap();
    assert!(
        extra_args.contains("building") && extra_args.contains("takes no arguments"),
        "unexpected output: {extra_args}"
    );

    // Anything not configured is an error.
    let err = run("rust", "lint", None).unwrap_err();
    assert!(
        err.to_string().contains("not configured"),
        "unexpected error: {err}"
    );
    let err = run("python", "build", None).unwrap_err();
    assert!(
        err.to_string().contains("not configured"),
        "unexpected error: {err}"
    );
}
