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
            # Build: gstreamer-1.0.pc and gstreamer-base-1.0.pc
            gst_all_1.gstreamer
            # Build: gstreamer-video-1.0.pc and gstreamer-gl-1.0.pc.
            # Runtime: playbin3, glsinkbin
            gst_all_1.gst-plugins-base
            # Runtime only: souphttpsrc, hlsdemux2
            gst_all_1.gst-plugins-good
            # Runtime only: hlsdemux, tsdemux, openh264dec, vah264dec
            gst_all_1.gst-plugins-bad
            # Runtime only: avdec_h264, avdec_aac
            gst_all_1.gst-libav
            # Runtime only: the GIO TLS backend. Bluesky's playlist URL is
            # https, so souphttpsrc/adaptivedemux2 go out through libsoup3,
            # which asks GIO for a GTlsBackend. Nothing else in this closure
            # provides one, and without it every video fails at runtime with
            # "TLS support is not available" while nix build still passes.
            glib-networking
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

          # wrapGAppsHook4 does not carry the GStreamer plugin path, so without
          # this the app builds and starts but no pipeline can find
          # souphttpsrc or hlsdemux2, and video fails at runtime only.
          #
          # GIO_EXTRA_MODULES is spelled out for the same reason: the TLS
          # backend is a runtime-only lookup, so getting it wrong is invisible
          # until someone opens an https stream.
          preFixup = ''
            gappsWrapperArgs+=(--prefix GST_PLUGIN_SYSTEM_PATH_1_0 : "$GST_PLUGIN_SYSTEM_PATH_1_0")
            gappsWrapperArgs+=(--prefix GIO_EXTRA_MODULES : "${pkgs.glib-networking}/lib/gio/modules")
          '';

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

          # Useful for VS Code / rust-analyzer to find the right headers.
          # The wrapper is what sets these for the built package; a `cargo run`
          # from this shell is unwrapped, so it needs them too or video fails
          # here and only here.
          shellHook = ''
            export RUST_BACKTRACE=1
            export GIO_EXTRA_MODULES="${pkgs.glib-networking}/lib/gio/modules''${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
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
