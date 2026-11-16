{ pkgs, lib, config, ... }:
let
  home = builtins.getEnv "HOME";
  # One sccache server per workspace root. A server reads SCCACHE_BASEDIRS once at start.
  # A shared server would keep the root of the first workspace that started it.
  port = 20000 + lib.mod (lib.fromHexString (builtins.substring 0 4 (builtins.hashString "sha256" config.devenv.root))) 20000;
in
{
  packages = [
    pkgs.git
    pkgs.cargo-nextest
    pkgs.sccache
  ];
  languages.rust.enable = true;

  # sccache shares compiled external crates and C code between checkouts.
  # Without it, each new workspace and each Testee slot builds them cold.
  # sccache keys on the working directory of rustc, which is the workspace root.
  # SCCACHE_BASEDIRS strips that root, so every workspace shares one cache.
  # devenv.root is the root of the workspace that runs this devenv.
  # sccache does not cache bin, proc-macro, or incremental crates.
  assertions = [
    {
      assertion = home != "";
      message = "HOME is empty, so SCCACHE_DIR has no safe path.";
    }
  ];
  env = {
    RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
    SCCACHE_DIR = "${home}/.local/state/sccache/scippy";
    SCCACHE_CACHE_SIZE = "20G";
    SCCACHE_BASEDIRS = config.devenv.root;
    SCCACHE_SERVER_PORT = toString port;
  };

  # Testee runs the standard tasks (check:*, test:*).
  # Every task declares `input.testee`, so Testee can reuse a verdict
  # when a commit changes none of the task inputs.
  # Run `testee doctor` after any edit to this table.
  # `scripts/verify.sh` runs `testee verify --profile quick` for the edit loop.
  # `test:full` runs the fast set. Slow tests carry `#[ignore = "slow: ..."]`.
  # `opt:slow` runs them. It does not count for GREEN.
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

      # The fast set: every test that has no `#[ignore]`. It is the required gate.
      # Nextest skips the ignored tests. `opt:slow` runs them.
      "test:full" = {
        exec = ''
          ${cargoEnv}
          ${scipEnv}
          cargo nextest run --workspace --profile ci
          cargo test --workspace --doc
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = hangGuard;
      };

      # The slow set: end-to-end tests that call the scip CLI, and the cache tests.
      # `--run-ignored only` runs the 27 tests that carry `#[ignore]`. Doctests have none.
      # Run it with `testee verify opt:slow` when a change touches indexing,
      # resolution, the scip output, or the metalayer cache.
      "opt:slow" = {
        exec = ''
          ${cargoEnv}
          ${scipEnv}
          cargo nextest run --workspace --profile ci --run-ignored only
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = hangGuard;
      };
    };
}
