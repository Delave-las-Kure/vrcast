# A throwaway server for the integration tests (T026).
#
# Ubuntu 24.04 — the same system as on the live server and on the test VPS. That is not a
# formality: checking the server-access layer on a different system means checking something
# other than what the application will work with.
#
# The container does NOT replace a real server entirely. It covers file operations, running
# commands, parsing the catalogue and transfers with breaks. It does not cover: systemd
# services, the network filter, kernel settings and issuing a certificate — those need a real
# VPS (Phase 7, see research.md R-06 and quickstart.md).

FROM ubuntu:24.04

# iproute2 provides ss: the channel-multiplexing test counts the connections by the server's
# own means. Without ss that count quietly degenerated into taking our word for it.
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      openssh-server \
      ca-certificates \
      iproute2 \
 && rm -rf /var/lib/apt/lists/*

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
RUN mkdir -p /var/lib/vrcast/videos /etc/vrcast

EXPOSE 22
CMD ["/usr/sbin/sshd", "-D", "-e"]
