# T245 — somebody else's server: a machine that already serves something, and not ours.
#
# FR-132 says the application must refuse to deploy or change anything here and must say what
# it recognised. That promise is worth exactly as much as the finding behind it, so the
# finding has to be real: a running web server with its own site, put there by somebody who
# never heard of this application.
#
# **Why nginx and not Caddy.** The recognition rule (contracts/server-contract.md) has two
# branches: `/etc/caddy/Caddyfile` is there, **or** a web server is running. A detector
# written in a hurry checks only the first — it is the one named in the path — and then a
# server running anything else is reported as clean and deployed over. nginx is the case that
# catches that mistake; the other branch is covered by the ordinary throwaway container,
# which carries a Caddyfile.
#
# There is no /etc/vrcast/state.json here and there must never be one: with it the server
# would be Managed, and this image would stop being what it is for.

FROM ubuntu:24.04

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      systemd systemd-sysv \
      openssh-server \
      nginx \
      curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*

RUN systemctl mask \
      systemd-udevd.service systemd-udev-trigger.service \
      systemd-modules-load.service \
      systemd-firstboot.service \
      systemd-resolved.service

# Somebody's site, with nothing about it resembling ours. The content matters only in that
# the refusal has to name what was found, and "an nginx serving /srv/somebodys-site" is a
# thing a person can act on, where "a foreign configuration" is not.
RUN mkdir -p /srv/somebodys-site \
 && printf '%s\n' \
      '<!doctype html>' \
      '<title>Somebody else was here first</title>' \
      '<h1>This machine already serves something.</h1>' \
      > /srv/somebodys-site/index.html \
 && printf '%s\n' \
      'server {' \
      '    listen 80 default_server;' \
      '    listen [::]:80 default_server;' \
      '    root /srv/somebodys-site;' \
      '    index index.html;' \
      '    server_name _;' \
      '}' \
      > /etc/nginx/sites-available/default \
 && nginx -t

RUN echo 'root:test-container-password' | chpasswd

RUN mkdir -p /run/sshd \
 && printf '%s\n' \
      'PermitRootLogin yes' \
      'PasswordAuthentication yes' \
      'PubkeyAuthentication yes' \
      'UsePAM no' \
      'PrintMotd no' \
      > /etc/ssh/sshd_config.d/99-fresh.conf \
 && systemctl enable ssh \
 && systemctl enable nginx

STOPSIGNAL SIGRTMIN+3

EXPOSE 22 80
CMD ["/sbin/init"]
