#!/bin/sh
# The systemd stand from integration/deploy_upgrade_v1.rs, never a real host.
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get -o Acquire::Retries=3 update -qq
apt-get -o Acquire::Retries=3 install -y -qq ffmpeg curl tar ufw ca-certificates gnupg unattended-upgrades >/dev/null
curl -fsSL --retry 3 --connect-timeout 20 -o /root/caddy-v1.deb https://github.com/caddyserver/caddy/releases/download/v2.11.4/caddy_2.11.4_linux_amd64.deb
dpkg -i /root/caddy-v1.deb >/dev/null
id vrcast >/dev/null 2>&1 || useradd --system --home /var/lib/vrcast --shell /usr/sbin/nologin vrcast
mkdir -p /var/lib/vrcast/videos/film /opt/vrcast /etc/vrcast /var/log/caddy /root/.ssh
cp /root/tour-authorized-keys /root/.ssh/authorized_keys
chmod 700 /root/.ssh
chmod 600 /root/.ssh/authorized_keys
cp /root/tour-Caddyfile.v1 /etc/caddy/Caddyfile
printf '%s\n' '# The quality-limit rules. This file belongs to VRCast Studio: it is rewritten whole' '# on every change, and anything added here by hand will be lost.' > /etc/caddy/vrcast-limits.conf
cp /root/tour-state.json /etc/vrcast/state.json
printf '#EXTM3U\n' > /var/lib/vrcast/videos/film/master.m3u8
printf 'the old piece' > /var/lib/vrcast/videos/film/seg_00001.ts
chown -R vrcast:vrcast /var/lib/vrcast /opt/vrcast
chown -R caddy:caddy /var/log/caddy
systemctl enable --now caddy unattended-upgrades
systemctl reload caddy
curl -1sLf --connect-timeout 20 https://dl.cloudsmith.io/public/caddy/stable/gpg.key | gpg --batch --yes --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf --connect-timeout 20 https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt > /etc/apt/sources.list.d/caddy-stable.list
curl -sk -D - --max-time 10 --resolve vrcast-upgrade.localhost:443:127.0.0.1 https://vrcast-upgrade.localhost/videos/film/seg_00001.ts
