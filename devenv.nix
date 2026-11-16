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
      # The timeouts cover a cold Cargo build in a new Testee workspace.
      # Measure a warm run, then set each timeout to about twice that time.
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
      };

      "check:lint" = {
        exec = ''
          ${cargoEnv}
          cargo clippy --workspace --all-targets -- -D warnings
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = 900;
      };

      "check:types" = {
        exec = ''
          ${cargoEnv}
          cargo check --workspace --all-targets
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = 900;
      };

      "test:quick" = {
        exec = ''
          ${cargoEnv}
          ${scipEnv}
          cargo test --workspace --lib
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = 900;
      };

      "test:full" = {
        exec = ''
          ${cargoEnv}
          ${scipEnv}
          cargo test --workspace
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = 1800;
      };
    };
}
