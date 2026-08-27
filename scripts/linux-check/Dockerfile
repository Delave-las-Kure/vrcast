# The machine every Linux check runs on — the developer's own and continuous integration's
# alike.
#
# It began as the first of those: code under `#[cfg(unix)]` does not compile on Windows at all,
# and any mistake in it stayed invisible until a run in CI. Ten minutes of waiting for a
# needless import is a bad trade.
#
# **Since 2026-08-27 it is also the image CI itself runs in**, and that is not tidiness. The
# checks moved onto a self-hosted runner — the owner's own machine — and a job promptly did
# what jobs do on the disposable machines they were written for: `dtolnay/rust-toolchain`
# installed rustup over the real one and took `rustup.exe` with it. Every shim in `.cargo/bin`
# points at rustup, so the toolchain went at once, along with `cargo-deny` and `tauri-driver`.
# Nothing was lost for good, and nothing should have been at risk.
#
# One image rather than two, and for the reason the old note here already gave: a second image
# would drift from this one, and a check running on something other than what it claims is
# reassuring and useless. So what CI needs was added here instead.
#
# The Rust version is deliberately NOT pinned — the image now IS the toolchain, and pinning it
# would freeze what every Linux check compiles with until somebody remembered to bump it.
# Proved once already on 1.90 against 1.98, where the old clippy complained about what the new
# one no longer counts an error. A false alarm in a check is worse than no check: people stop
# looking at it.

FROM rust:bookworm

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      libwebkit2gtk-4.1-dev \
      libappindicator3-dev \
      librsvg2-dev \
      patchelf \
      pkg-config \
 && rm -rf /var/lib/apt/lists/*

# What continuous integration needs on top of the developer's check.
#
# `webkit2gtk-driver` and `xvfb` are the end-to-end harness: it drives a real window, and a
# machine with no display has nowhere to open one. `docker.io` is the client only — the
# integration checks bring containers up on the machine's own daemon through its socket, and a
# daemon inside a daemon would be both slower and a different thing from what is checked.
# `git` is what checking out the sources needs; `python3`, the theme check.
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      docker.io \
      git \
      python3 \
      unzip \
      webkit2gtk-driver \
      xvfb \
 && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy

# The two cargo tools the checks use, built in rather than installed per run: minutes each
# time, and on a self-hosted runner an install would land in a real `~/.cargo`. Pinned for the
# same reason the image is named with a number — an unpinned tool changes what was checked
# without anybody choosing to change it.
RUN cargo install cargo-deny --locked --version 0.20.2 \
 && cargo install tauri-driver --locked --version 2.0.6 \
 && rm -rf "$CARGO_HOME/registry"

# The build directory lives inside the image rather than in the mounted sources: otherwise
# the Linux and Windows builds would wipe out each other's artefacts.
ENV CARGO_TARGET_DIR=/build/target
WORKDIR /app/src-tauri
