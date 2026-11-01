#!/usr/bin/env python3
"""Diagnose whether Docker preserves a random published endpoint across a real cut.

Only disposable resources made by this probe are touched. No application profiles or secrets.
"""
import json
import subprocess
import uuid


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True, timeout=90).strip()


def main():
    name = 'vrcast-t721-probe-' + uuid.uuid4().hex[:12]
    network = name + '-net'
    container = None
    docker('network', 'create', network)
    try:
        container = docker('run', '-d', '--rm', '--init', '--name', name,
                           '--network', network, '-p', '127.0.0.1::22', 'ubuntu:24.04', 'sleep', '300')
        before = docker('port', container, '22/tcp')
        docker('network', 'disconnect', '-f', network, container)
        docker('network', 'connect', network, container)
        after = docker('port', container, '22/tcp')
        print(json.dumps({'published_before_cut': before, 'published_after_restore': after,
                          'same_endpoint': before == after}), flush=True)
    finally:
        if container:
            docker('rm', '-f', container)
        docker('network', 'rm', network)


if __name__ == '__main__':
    main()
