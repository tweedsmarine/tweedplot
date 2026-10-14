# Setting up the Windows PC

About 30–45 minutes, mostly downloads. Do the steps in this order.

## 1. Visual Studio Build Tools (C++ compiler)

Rust on Windows and Tauri both need Microsoft's C++ build tools.

1. Download **Build Tools for Visual Studio 2022** from https://visualstudio.microsoft.com/visual-cpp-build-tools/
2. In the installer, tick **Desktop development with C++**. Keep the default options (MSVC and the Windows 11 SDK).
3. **ARM64 PCs (Snapdragon etc.):** also go to **Individual components** and tick **MSVC v143 – VS 2022 C++ ARM64/ARM64EC build tools (Latest)**. Without it, Rust can't find Microsoft's linker.
4. Install (several GB), then restart if it asks you to.

**Run `cargo` from PowerShell, not Git Bash.** Git Bash has its own `link` command (a GNU file tool). If Rust can't find the Microsoft linker, it picks that one up by mistake and fails with "link: extra operand".

**Keep the repo out of MEGA** (e.g. `C:\dev\racenav`). The `target` build folder grows to thousands of files and hundreds of MB, and MEGA would try to sync all of it.

## 2. Rust

1. Download `rustup-init.exe` from https://rustup.rs
2. Run it and accept the defaults (toolchain `stable-x86_64-pc-windows-msvc`).
3. Open a **new** PowerShell window and check:

```
rustc --version
cargo --version
```

## 3. Node.js (for the map front end and Tauri)

1. Install the **LTS** version from https://nodejs.org
2. Check it in a new PowerShell window: `node --version` and `npm --version`

WebView2, which Tauri uses to show the app window, comes with Windows 10 and 11, so there's nothing to install.

## 4. Git and GitHub

1. Install **GitHub Desktop** (https://desktop.github.com), which includes git, or Git for Windows.
2. On github.com, create a **private** repository called `racenav`. Leave it empty: no README and no licence.
3. Clone the bundle Claude sent you, then point it at GitHub:

```
git clone racenav.bundle racenav
cd racenav
git remote set-url origin https://github.com/<your-user>/racenav.git
git push -u origin main
```

4. Run the tests: `cargo test`. They should all pass.

## 5. Signal K server (GPS input)

1. In PowerShell: `npm install -g signalk-server`
2. Start it with `signalk-server` and open http://localhost:3000. Create an admin user when it asks.
3. **Live GPS:** Server → Data Connections → Add. Input type **NMEA 0183**, source **Serial**. Pick the GPS's COM port (Device Manager → Ports) and baud rate (usually 4800 or 9600).
4. **Desk testing without a GPS:** add a second data connection of type **File Stream** and point it at one of the Morgane `.nmea` logs. It plays the voyage back as if it were live.
5. Check the **Data Browser** page: `navigation.position` should be updating.

The racenav app connects to `ws://localhost:3000/signalk/v1/stream`, so anything Signal K shows, the app can show too.
