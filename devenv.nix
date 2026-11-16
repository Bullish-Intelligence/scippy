{ pkgs, ... }:
{
  packages = [ pkgs.git ];
  languages.rust.enable = true;

  # Testee runs the standard tasks (check:*, test:*).
  # Every task declares `input.testee`, so Testee can reuse a verdict
  # when a commit changes none of the task inputs.
  # Run `testee doctor` after any edit to this table.
  # `scripts/verify.sh` runs `testee verify --profile quick` for the edit loop.
  # Testee runs `git clean -ffdx -e .devenv` in its slot, which deletes `target/`.
  # Each Cargo task sets `CARGO_TARGET_DIR` under `.devenv/`, so the build stays warm.
  tasks =
    let
      # Files that no check or test task reads.
      # Tests mention scripts/ and README.md only in comments and temporary files.
      unread = [
        "README.md"
        "AGENTS.md"
        "CLAUDE.md"
        ".gitignore"
        ".tmuxp.yaml"
        ".copyroom-local.json"
        "scripts/**"
      ];
      # Keep the Cargo target dir under `.devenv/`. The slot clean keeps that path.
      cargoEnv = ''
        export CARGO_TARGET_DIR="''${DEVENV_STATE:?DEVENV_STATE is not set}/cargo-target"
      '';
      # A Testee timeout is a hang guard, not a speed check.
      # Tasks run under any machine load, so each limit is 4 hours.
      # Put benchmarks in an opt-in `bench:*` task, never in a check or test task.
      # Run one with `testee verify bench:<name>`. Its verdict never counts toward green.
      hangGuard = 14400;
      # The tests call the scip CLI. Make them fail, not skip, when it is missing.
      scipEnv = ''
        scip_dir="''${SCIP_BIN_DIR:-$HOME/.local/state/scippy/tools/scip/v0.10.0/bin}"
        PATH="$scip_dir:$PATH"
        export SCIPPY_REQUIRE_SCIP=1
        command -v scip >/dev/null || { echo "scip CLI not found in $scip_dir" >&2; exit 1; }
      '';
    in
    {
      "check:format" = {
        exec = "cargo fmt --check";
        input.testee.ignores = unread;
        input.testee.timeout_s = hangGuard;
      };

      "check:lint" = {
        exec = ''
          ${cargoEnv}
          cargo clippy --workspace --all-targets -- -D warnings
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = hangGuard;
      };

      "check:types" = {
        exec = ''
          ${cargoEnv}
          cargo check --workspace --all-targets
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = hangGuard;
      };

      "test:quick" = {
        exec = ''
          ${cargoEnv}
          ${scipEnv}
          cargo test --workspace --lib
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = hangGuard;
      };

      "test:full" = {
        exec = ''
          ${cargoEnv}
          ${scipEnv}
          cargo test --workspace
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = hangGuard;
      };
    };
}
