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
  # `opt:slow` runs them. It runs only on explicit request, and it does not count for GREEN.
  # `opt:slow` also holds the performance and load-sensitive tests: `#[ignore = "perf: ..."]`.
  # A test that asserts on time, speed, or resource use never joins the default set.
  # Testee runs `git clean -ffdx -e .devenv` in its slot, which deletes `target/`.
  # Each Cargo task sets `CARGO_TARGET_DIR` under `.devenv/`, so the build stays warm.
  # The test tasks use the Cargo profile `gate` (see Cargo.toml): scippy-core at opt-level 2.
  # `gate` builds into `gate/` next to `debug/`, so the lint and check builds stay warm too.
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
      # A Testee timeout is a hang guard: it is about twice the worst measured time.
      # Measured 2026-11-16 from Testee run records since the fast gate (7b9e484):
      # lint 250 s under load (64 s cold slot, warm sccache, load ~10), format 7 s,
      # types and quick take the lint cold-slot time (warm runs took 5 s and 45 s),
      # test:full 387 s, opt:slow 753 s, bench:identity 302 s.
      # A timeout must not trip under the normal shared-host load (15 to 45 on 8 cores).
      # When a measured time changes, update the limit and this comment.
      # Put benchmarks in an opt-in `bench:*` task, never in a check or test task.
      # Run one with `testee verify bench:<name>`. Its verdict never counts toward green.
      timeoutFormat = 60;
      timeoutCompile = 600;
      timeoutFull = 800;
      timeoutSlow = 1500;
      timeoutBench = 600;
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
        input.testee.timeout_s = timeoutFormat;
      };

      "check:lint" = {
        exec = ''
          ${cargoEnv}
          cargo clippy --workspace --all-targets -- -D warnings
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = timeoutCompile;
      };

      "check:types" = {
        exec = ''
          ${cargoEnv}
          cargo check --workspace --all-targets
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = timeoutCompile;
      };

      "test:quick" = {
        exec = ''
          ${cargoEnv}
          ${scipEnv}
          cargo test --profile gate --workspace --lib
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = timeoutCompile;
      };

      # The fast set: every test that has no `#[ignore]`. It is the required gate.
      # Nextest skips the ignored tests. `opt:slow` runs them.
      "test:full" = {
        exec = ''
          ${cargoEnv}
          ${scipEnv}
          cargo nextest run --cargo-profile gate --workspace --profile ci
          cargo test --profile gate --workspace --doc
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = timeoutFull;
      };

      # The slow set: end-to-end tests that call the scip CLI, and the cache tests.
      # It also holds the performance and load-sensitive tests. Nextest runs every ignored test.
      # `--run-ignored only` runs the 27 tests that carry `#[ignore]`. Doctests have none.
      # It runs only on explicit request. No lane verify step runs it.
      "opt:slow" = {
        exec = ''
          ${cargoEnv}
          ${scipEnv}
          cargo nextest run --workspace --profile ci --run-ignored only
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = timeoutSlow;
      };

      # Opt-in benchmark. It is not a check or a test. Run it with `testee verify bench:identity`.
      "bench:identity" = {
        exec = ''
          ${cargoEnv}
          export PATH="${pkgs.python3}/bin:$PATH"
          scripts/identity-eval.sh overhead
        '';
        input.testee.ignores = unread;
        input.testee.timeout_s = timeoutBench;
      };
    };
}
