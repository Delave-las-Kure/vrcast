#!/usr/bin/env python3
"""Compact read-only summaries of saved GitHub Actions API JSON (no credentials)."""
import json
import sys

with open(sys.argv[1], encoding="utf-8") as source:
    data = json.load(source)
for job in data.get("jobs", []):
    print(job["id"], job["name"], job["status"], job["conclusion"])
    for step in job.get("steps", []):
        if step["conclusion"] in ("failure", "cancelled"):
            print("  ", step["name"], step["conclusion"])
for run in data.get("workflow_runs", []):
    print(run["id"], run["head_sha"], run["status"], run["conclusion"], run["html_url"])
