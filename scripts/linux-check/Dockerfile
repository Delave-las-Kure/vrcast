# The image for checking the Linux build from a developer's machine.
#
# It exists for one class of fault: code under `#[cfg(unix)]` does not compile on Windows at
# all, and any mistake in it is invisible until a run in continuous integration. Ten minutes
# of waiting for a needless import is a bad trade.
#
# The contents mirror what CI installs (.github/workflows/build.yml): a divergence between
# this image and CI would make the check reassuring but useless.
#
# The version is deliberately NOT pinned. CI takes `dtolnay/rust-toolchain@stable`, that is,
# always the latest stable; a number pinned here would drift from it and start giving remarks
# CI does not have — proved on 1.90 against 1.98, where the old clippy complained about what
# the new one no longer counts an error. A false alarm in a check is worse than no check:
# people stop looking at it.

FROM rust:bookworm

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      libwebkit2gtk-4.1-dev \
      libappindicator3-dev \
      librsvg2-dev \
      patchelf \
      pkg-config \
 && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy

# The build directory lives inside the image rather than in the mounted sources: otherwise
# the Linux and Windows builds would wipe out each other's artefacts.
ENV CARGO_TARGET_DIR=/build/target
WORKDIR /app/src-tauri
