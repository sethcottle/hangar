{
  description = "Hangar - A native Bluesky client for Linux";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  inputs.flake-parts.url = "github:hercules-ci/flake-parts";

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = ["x86_64-linux" "aarch64-linux"];

      perSystem = {
        config,
        pkgs,
        system,
        ...
      }: let
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

        hangar = pkgs.rustPlatform.buildRustPackage {
          pname = "hangar";
          version = cargoToml.package.version;
          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          buildInputs = with pkgs; [
            gtk4
            libadwaita
            libsoup_3
            libsecret
            openssl
            sqlite
          ];

          nativeBuildInputs = with pkgs; [
            blueprint-compiler
            desktop-file-utils
            glib
            pkg-config
            wrapGAppsHook4
          ];

          buildType = "release";
          strictDeps = true;

          RUST_BACKTRACE = 1;

          meta = {
            description = "A native Bluesky client for Linux";
            mainProgram = "hangar";
            platforms = pkgs.lib.platforms.linux;
            license = pkgs.lib.licenses.mpl20;
          };
        };
      in {
        packages = {
          default = hangar;
          inherit hangar;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [config.packages.hangar];

          # Useful for VS Code / rust-analyzer to find the right headers
          shellHook = ''
            export RUST_BACKTRACE=1
          '';

          packages = with pkgs; [
            clippy
            rust-analyzer
            rustfmt
          ];
        };
      };
    };
}
