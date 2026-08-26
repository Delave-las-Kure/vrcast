#!/bin/sh
# Both services in one container (T149).
#
# The application reaches the server over SSH and manages the web server there — exactly as
# it does on a real one. Splitting them into two containers would mean the application could
# not restart the web server, and that is half of what milestone C checks: applying a
# quality limit ends with a reload, and its rollback with another one.

# The web server first, and its own output into a file rather than into the container's log.
# `docker logs` is read by the tests as sshd's log (fixture.rs, wait_in_sshd_log), and a
# second talkative service in there would turn searching for a line into guesswork.
if ! caddy start --config /etc/caddy/Caddyfile --adapter caddyfile \
        > /var/log/caddy/caddy.log 2>&1; then
	# Into the container's log deliberately: without this the failure reaches the tests as
	# "the server did not start accepting connections", which says nothing about the cause.
	echo "the web server would not start. Its log:" >&2
	cat /var/log/caddy/caddy.log >&2
	exit 1
fi

# sshd in the foreground: it is the container's main process, and its output is what
# `docker logs` shows.
exec /usr/sbin/sshd -D -e
