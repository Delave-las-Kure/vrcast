# A throwaway server for the integration tests (T026, T149).
#
# Ubuntu 24.04 — the same system as on the live server and on the test VPS. That is not a
# formality: checking the server-access layer on a different system means checking something
# other than what the application will work with.
#
# The container does NOT replace a real server entirely. It covers file operations, running
# commands, parsing the catalogue, transfers with breaks and — since T149 — serving files
# over HTTP with an access log, which is what viewers and quality limits are read out of.
# It does not cover: systemd services, the network filter, kernel settings and issuing a
# certificate — those need a real VPS (Phase 7, see research.md R-06 and quickstart.md).

FROM ubuntu:24.04

# ffmpeg is here because the SERVER cuts the segments, not this machine (T196): hundreds
# of small files are born where they will be served rather than dragged across the network
# one by one. The live server gets it the same way, from apt, at deployment — so checking
# against a container without it would be checking something the application never does.
# iproute2 provides ss: the channel-multiplexing test counts the connections by the server's
# own means, and a viewer's delivery speed is read from that same table (R-02). Without ss
# that count quietly degenerated into taking our word for it.
# curl and ca-certificates are here for fetching the web server below, and curl is also what
# the viewer helpers pull with (T152). procps gives pgrep and pkill: the helpers have to say
# whether a viewer is still pulling, and reading that out of /proc by hand works but reads
# like a puzzle.
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      openssh-server \
      ca-certificates \
      ffmpeg \
      curl \
      iproute2 \
      procps \
 && rm -rf /var/lib/apt/lists/*

# Caddy — the same web server as on the live one, and it is not interchangeable here. The
# checks of milestone C rest on things only Caddy has: the shape of its access log, its
# refusal to accept a broken configuration, and a reload that leaves the previous one
# serving when the new one is rejected. Another server would check something else and say
# nothing about what will actually run.
#
# Taken by an exact version with the checksum verified — the same way as the FFmpeg build
# (scripts/ffmpeg.json). "Whatever came back from the address" is not a foundation to build
# a check on. The checksum is SHA-512: that is what this project publishes.
ARG CADDY_VERSION=2.11.4
ARG CADDY_SHA512=8220d1f013b6f27510247b2360c9e0ca9f018feebd82515f07635318b34ff9777ccc8fd0b6e6f2486ce3a33fe389fbb7db12d05baa474f4587509fb4f5ebf1c9
RUN curl -fsSL -o /tmp/caddy.tar.gz \
      "https://github.com/caddyserver/caddy/releases/download/v${CADDY_VERSION}/caddy_${CADDY_VERSION}_linux_amd64.tar.gz" \
 && echo "${CADDY_SHA512}  /tmp/caddy.tar.gz" | sha512sum -c - \
 && tar -xzf /tmp/caddy.tar.gz -C /usr/local/bin caddy \
 && rm /tmp/caddy.tar.gz \
 && caddy version

RUN mkdir -p /run/sshd /root/.ssh \
 && chmod 700 /root/.ssh

# The public half of the same key the passphrase-protected key parsing is checked with. That
# way the integration test also carries FR-096 through to the end: not "the key parsed" but
# "it got us into a real OpenSSH".
COPY encrypted_ed25519.key.pub /root/.ssh/authorized_keys
RUN chmod 600 /root/.ssh/authorized_keys

# The password is needed to check both the second way in and its refusal. It is also in
# tests/integration/fixture.rs, and the two must be changed together.
RUN echo 'root:test-container-password' | chpasswd

RUN printf '%s\n' \
      'PermitRootLogin yes' \
      'PasswordAuthentication yes' \
      'PubkeyAuthentication yes' \
      'UsePAM no' \
      'PrintMotd no' \
      > /etc/ssh/sshd_config.d/99-test.conf

# The serving directory in the same place as on a real server: the paths take part in the
# checks, and substituting them would check a different case.
RUN mkdir -p /var/lib/vrcast/videos /etc/vrcast /etc/caddy /var/log/caddy

# The file the application owns (R-03), with no rules in it to begin with. It has to exist
# even so: importing a pattern that matches nothing is an error, Caddy would refuse to
# start, and every check of milestone C would fail for a reason having nothing to do with
# the code. And it holds a line rather than nothing at all, because an empty import draws a
# warning on every validate — and a warning that is always there teaches people not to read
# warnings.
RUN printf '%s\n' \
      '# The quality-limit rules. This file belongs to VRCast Studio: it is rewritten whole' \
      '# on every change, and anything added here by hand will be lost.' \
      > /etc/caddy/vrcast-limits.conf

COPY Caddyfile /etc/caddy/Caddyfile
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh \
 && caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile

EXPOSE 22 80
CMD ["/usr/local/bin/entrypoint.sh"]
