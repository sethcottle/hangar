# Hangar

A native Bluesky client for Linux, built with Rust, GTK4, and Libadwaita.

## Building

### Dependencies

**Fedora/RHEL:**
```bash
sudo dnf install gtk4-devel libadwaita-devel gcc pkg-config
```

**Ubuntu/Debian:**
```bash
sudo apt install libgtk-4-dev libadwaita-1-dev build-essential pkg-config
```

**Arch:**
```bash
sudo pacman -S gtk4 libadwaita base-devel
```

### Build & Run

```bash
cargo build --release
cargo run --release
```

## License

MPL-2.0
