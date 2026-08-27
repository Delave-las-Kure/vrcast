# T244 — a bare server, the way one arrives from a hosting provider.
#
# This is the only state a deployment can be checked from, and the existing throwaway
# container (fixtures/docker) cannot stand in for it: that one already carries a web server,
# the serving directories and the rules file, so the application is *supposed* to recognise
# it as deployed or as somebody else's. Checking "recognised as clean" against it would
# check the opposite of what it says.
#
# What is deliberately absent: Caddy, ffmpeg, MediaMTX, /var/lib/vrcast, /etc/vrcast,
# /etc/caddy. Every one of them is something a deployment step has to create, and a step
# whose work is already done proves nothing about the step.
#
# What is deliberately present: everything a stock Ubuntu cloud image carries. A container
# stripped below that would make the `packages` step install things that on a real VPS are
# already there — again checking something other than what will happen.

FROM ubuntu:24.04

# systemd, because a bought VPS has it and the deployment talks to it: services are enabled
# and started through it, and so is the network filter's own unit.
#
# **Measured 2026-08-27, T246** — what a privileged container with systemd can and cannot do,
# established by running it rather than by reasoning about it:
#
#   works:  systemd as PID 1; systemctl enable / start / restart / is-active; a real sshd
#           and its hardening; ufw, with 98 IPv4 rules and 137 IPv6 rules installed — so the
#           parity FR-136 demands is checkable here;
#   fails:  `swapon` — "Invalid argument" whatever the privileges, and worse, `free` inside
#           the container reports the **host's** swap, so a naive check would pass on a
#           server that has none;
#   fails:  net.core.rmem_max, net.core.wmem_max, net.core.default_qdisc — these keys are not
#           per-network-namespace, so they are invisible inside any container on any host,
#           privileged or not. Only somaxconn and the xfrm keys are namespaced;
#   host-dependent: net.ipv4.tcp_congestion_control exists and is writable, but only to an
#           algorithm the **host's** kernel carries. Under Docker Desktop on WSL2 that is
#           `reno cubic` — no bbr, and no sch_fq module either, so `tc qdisc ... fq` fails;
#   fails:  udev — /sys/block holds only loop devices, so the readahead rule has nothing to
#           apply to.
#
# Hence the rule the step mechanism is built on: a step's check has a third answer besides
# applied and failed — **not checkable here**. Without it a container run would report a
# fully deployed server that has neither swap nor tuning.
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      systemd systemd-sysv \
      openssh-server \
      ufw iproute2 procps kmod \
      curl ca-certificates tar sudo \
 && rm -rf /var/lib/apt/lists/*

# Units that cannot work without real hardware, or that fight the container's own
# arrangements. Masked rather than disabled: a disabled unit can still be pulled in by
# another one's dependencies, and then the boot hangs on it.
RUN systemctl mask \
      systemd-udevd.service systemd-udev-trigger.service \
      systemd-modules-load.service \
      systemd-firstboot.service \
      systemd-resolved.service

# The password and nothing else. **No authorized_keys on purpose**: a freshly bought server
# has only what the provider put there, and the application's own `ssh-key` step is what puts
# the key in. Handing the container a key would leave that step with nothing to do and its
# check with nothing to see — and it is the step the whole ordering rule exists for
# (ssh-key before ssh-hardening, R-12).
#
# The password matches tests/integration/fixture.rs; the two must be changed together.
RUN echo 'root:test-container-password' | chpasswd

# What a provider's image leaves behind: root may log in, by password. This is the state the
# hardening step is supposed to take away, so it has to be the state we start from.
RUN mkdir -p /run/sshd \
 && printf '%s\n' \
      'PermitRootLogin yes' \
      'PasswordAuthentication yes' \
      'PubkeyAuthentication yes' \
      'UsePAM no' \
      'PrintMotd no' \
      > /etc/ssh/sshd_config.d/99-fresh.conf \
 && systemctl enable ssh

# systemd wants this to shut down rather than be killed; without it every `docker stop`
# waits out the full timeout.
STOPSIGNAL SIGRTMIN+3

EXPOSE 22
CMD ["/sbin/init"]
