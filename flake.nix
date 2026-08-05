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
            # Build: egl.pc, which gstreamer-gl-egl-1.0.pc requires. The
            # sink's GL features pull that module in, and its own .pc ships
            # with gst-plugins-base while egl.pc does not. x11-xcb and
            # wayland-egl, wanted by the other two GL platform modules,
            # already arrive with libx11 and wayland.
            libGL
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
            # glib-compile-schemas, and the setup hook that puts the
            # installed schema where the wrapper looks for it
            glib
            pkg-config
            wrapGAppsHook4
          ];

          buildType = "release";
          strictDeps = true;

          # The update check offers a GitHub download, which is the wrong
          # answer for someone who installed from the flake: they update
          # with nix. The check needs APPIMAGE or /.flatpak-info to run at
          # all, so it is already quiet here; this labels the build honestly.
          HANGAR_DISTRIBUTION = "nix";

          # buildRustPackage installs the binary and stops there. Without
          # the rest of this a NixOS install has no launcher entry, and a
          # Wayland compositor cannot match the app_id to a desktop file,
          # so the window shows a generic icon and name in alt-tab.
          postInstall = ''
            install -Dm644 data/io.github.sethcottle.Hangar.desktop \
              -t $out/share/applications
            install -Dm644 data/io.github.sethcottle.Hangar.metainfo.xml \
              -t $out/share/metainfo
            install -Dm644 assets/icons/io.github.sethcottle.Hangar.svg \
              -t $out/share/icons/hicolor/scalable/apps

            # Raster sizes for desktops whose SVG renderer cannot draw the
            # scalable icon's filters (KDE turns them into black plates).
            for size in 16 32 48 64 128 256; do
              install -Dm644 "assets/icons/png/$size/io.github.sethcottle.Hangar.png" \
                -t "$out/share/icons/hicolor/''${size}x''${size}/apps"
            done

            # Window geometry lives in GSettings, and gio::Settings finds
            # nothing without the installed schema. Compiled here rather
            # than left to glib's fixup hook, which relocates this whole
            # directory under share/gsettings-schemas and recompiles it.
            install -Dm644 data/io.github.sethcottle.Hangar.gschema.xml \
              -t $out/share/glib-2.0/schemas
            glib-compile-schemas $out/share/glib-2.0/schemas
          '';

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
