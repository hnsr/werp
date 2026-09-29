# Publishing Werp for Fedora: plan

Status: plan only. No RPM or COPR repository has been created.

## What we are aiming for

Start with [COPR](https://docs.copr.fedorainfracloud.org/user_documentation.html), Fedora's service for community-maintained software repositories. We supply instructions for building Werp; COPR builds the app for selected Fedora versions and hosts the packages. For the full conversion features, users would first enable the separate RPM Fusion Free repository, then enable **our** repository and install with roughly:

```sh
sudo dnf copr enable OWNER/PROJECT
sudo dnf install werp
```

`OWNER/PROJECT` is a placeholder until a COPR project exists. The commands above assume RPM Fusion Free is already enabled; some systems may also need to replace Fedora's `ffmpeg-free` package with RPM Fusion's `ffmpeg` first. We will test and publish the exact steps. Users must choose to enable these repositories; Werp would not appear in Fedora's official repository. Official Fedora inclusion is possible later. It would require a Fedora maintainer and package review, so COPR is a sensible first milestone.

Some terms used below:

| Term | Meaning |
| --- | --- |
| RPM | Fedora's installable package format. A **binary RPM** contains the built app and installation metadata. |
| SRPM (source RPM) | The app's source plus a recipe for building the binary RPM. This is what we submit to COPR first. |
| Spec file | That recipe: it lists build and runtime dependencies, build commands, files to install, and package metadata. |
| Build environment / chroot | A clean, isolated Fedora installation used to build for one Fedora version and CPU architecture. |
| `mock` | A tool for testing the SRPM in such a clean environment on our own machine before using COPR. |

The first target should be Fedora 44 KDE on `x86_64`, since that is the development baseline in `DEVELOPMENT.md`. Add more Fedora releases and CPU architectures once each passes a clean build and installation test. Start with one package named `werp`, containing the KDE app (`werp-kde`), its private Rust helper (`werp-backend`), the diagnostic CLI (`werp`), and the two desktop menu entries. We can split the CLI into a separate package if that becomes useful.

## Work to do before a first COPR build

### 1. Finish the license check

Werp now has an MIT `LICENSE` file, and its Rust crates declare `MIT`. Before publishing, confirm that the project's contributors and assets can be released under those terms. Also check the licenses of the third-party Rust crates recorded in `Cargo.lock`. If their source is included in the SRPM, their notices must travel with it. [COPR's rules](https://docs.copr.fedorainfracloud.org/user_documentation.html#what-i-can-build-in-copr) require acceptable licensing and put responsibility for that check on the project owner. Choosing a license for Werp alone does not license its dependencies.

### 2. Make the installed app use release builds

The KDE app starts a separate Rust program, `werp-backend`, behind the scenes. Today CMake builds the helper in Cargo's **debug** mode and copies it from `target/debug`. That works for development, but an RPM needs a predictable way to build and install the optimized **release** helper alongside the KDE app. Add a release option to CMake, or have the spec file build and install the helper separately. Both programs must come from the same source version, and the GUI must find the helper after installation under `/usr`, without the source checkout present. Build and include the `werp` CLI too.

### 3. Gather the files and information users expect

- Create a source archive from a named Git release tag so we can reproduce exactly what was built. Keep the Cargo and CMake version numbers in sync and write short release notes.
- The app icon is installed under the `nl.hnsr.Werp` ID as SVG and PNGs from 16 to 512 pixels; check that it appears at menu and taskbar sizes on the clean system. Add [AppStream metadata](https://community.kde.org/Guidelines_and_HOWTOs/AppStream) under that same app ID so desktop software centers can describe Werp, and validate it and the desktop entries. The entries already advertise supported video file types, but that does not guarantee the installed app can decode every file.
- Keep the project's source, issue tracker, and support links easy to find. Document which Fedora versions we actually test.

### 4. Require an FFmpeg build with H.264 encoding

Werp calls the external `ffprobe` program to inspect video. It calls `ffmpeg` for conversion and some subtitle work. Fedora's [`ffmpeg-free`](https://packages.fedoraproject.org/pkgs/ffmpeg/ffmpeg-free/) supplies these programs but has a limited set of codecs, including no `libx264` encoder on the build we checked. [RPM Fusion Free publishes an `ffmpeg` package](https://download1.rpmfusion.org/free/fedora/updates/44/x86_64/repoview/ffmpeg.html); on the Fedora 44 development machine, that package provides `ffprobe` and `ffmpeg` with `libx264` and AAC encoding. Despite the repository name, RPM Fusion **Free** is a third-party repository, separate from Fedora's own repositories.

For the first full-featured release, plan to put `Requires: ffmpeg` in Werp's RPM spec and verify that it selects RPM Fusion's package rather than `ffmpeg-free`. COPR [supports external runtime repositories](https://docs.copr.fedorainfracloud.org/user_documentation.html#external-repositories), but the user-facing instructions should explicitly explain and test the RPM Fusion setup, including any needed `ffmpeg-free` replacement. A package dependency cannot by itself guarantee the encoder set forever, so run `ffmpeg -encoders` and a small H.264/AAC conversion in clean-install testing. Document failures when a user installs another FFmpeg build or codec support changes. Werp's COPR package will not bundle FFmpeg or codecs; [COPR's content rules](https://docs.copr.fedorainfracloud.org/user_documentation.html#what-i-can-build-in-copr) still apply to what we upload.

## Build and test the RPM

1. Write `packaging/fedora/werp.spec`. It should identify the tagged source, license, build tools (Rust/Cargo, C++/CMake/Ninja, Qt 6, KDE Frameworks 6), runtime dependencies including `ffmpeg`, build commands, and every installed file. Check exact Fedora dependency package names in the clean build environment. Install files into the RPM's temporary package directory, not the developer's home or `target/` directory.
2. Supply Cargo dependencies to the isolated build. The current `Cargo.lock` fixes crate versions; the binary RPM build should use `--locked --offline`. Prepare an audited source bundle with [`cargo vendor`](https://doc.rust-lang.org/cargo/commands/cargo-vendor.html), or use Fedora's [Rust packaging tools](https://docs.fedoraproject.org/en-US/packaging-guidelines/Rust/) if they handle this workspace. This keeps binary builds independent of a live crates.io download. Check that Fedora's Rust compiler is new enough for this code; `rust-toolchain.toml` currently pins 1.96.0 for local development.
3. Build an SRPM and binary RPM locally. Rebuild the SRPM with `mock` for the intended Fedora release. Check build logs, included files, runtime dependencies, license information, and `rpmlint` output. Run the Rust tests and offscreen KDE test that are reliable in a clean build; run tests needing special FFmpeg codecs separately.
4. Install the RPM on a fresh Fedora KDE system with Fedora, RPM Fusion Free, and the Werp COPR enabled. Verify that the `ffmpeg` requirement is resolved, including replacement of an existing `ffmpeg-free` installation where needed. Check the two menu actions, CLI help and file inspection, GUI-to-helper startup, H.264/AAC conversion, upgrade, and removal. If practical, do one separate manual playback check with an intended Cast receiver; automated package tests must not contact TVs.

## Publish through COPR

1. A maintainer creates a [Fedora Account](https://accounts.fedoraproject.org/), signs in to [COPR](https://copr.fedorainfracloud.org/), creates a Werp project, and enables only the Fedora build environment already tested locally. The web interface is enough to begin; `copr-cli` is optional and uses a token stored outside Git.
2. Upload the verified SRPM in COPR's web interface, or submit it with `copr-cli build OWNER/PROJECT path/to/werp.src.rpm`. COPR also supports building from Git, but an SRPM makes the exact first release input easy to inspect. Check the remote build log and install the resulting RPM from the COPR repository on a fresh system.
3. After that first release works, automate creating the SRPM from a release tag and submitting it. Keep stable releases tied to tags rather than every source push. Rebuild for supported Fedora versions when dependencies change, and publish tested RPM Fusion setup, installation, codec, and support instructions in the README.

## Completion criteria

- The contributor, asset, and dependency license audit permits a public COPR build.
- A tagged SRPM builds in local `mock` and COPR without downloading crates during the binary build.
- A clean Fedora KDE installation starts the GUI and its packaged helper, runs the CLI, and survives an upgrade.
- The `ffmpeg` dependency resolves from RPM Fusion Free, and H.264/AAC conversion works on the tested installation.
- Users can follow the published RPM Fusion, `dnf copr enable`, and `dnf install` instructions to install a public build.

## References

- [COPR user documentation: how COPR works, source types, rules, and builds](https://docs.copr.fedorainfracloud.org/user_documentation.html)
- [Fedora RPM packaging guide](https://rpm-packaging-guide.github.io/)
- [Fedora Rust packaging guidelines](https://docs.fedoraproject.org/en-US/packaging-guidelines/Rust/)
- [Cargo vendor documentation](https://doc.rust-lang.org/cargo/commands/cargo-vendor.html)
- [Fedora `ffmpeg-free` package](https://packages.fedoraproject.org/pkgs/ffmpeg/ffmpeg-free/)
- [RPM Fusion Free `ffmpeg` package](https://download1.rpmfusion.org/free/fedora/updates/44/x86_64/repoview/ffmpeg.html)
- [KDE AppStream guidance](https://community.kde.org/Guidelines_and_HOWTOs/AppStream)
