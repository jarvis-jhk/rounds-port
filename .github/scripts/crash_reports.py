"""Polls the crash-report ntfy topic and files GitHub issues.

The topic is public, so every message is untrusted: only messages shaped
like the game's reports are used, text goes into a code block, and a run
files at most MAX_NEW issues and MAX_COMMENTS comments.
"""

import json
import os
import re
import urllib.request

TOPIC = "https://ntfy.sh/rounds-port-crash-t2g6cbvu1b10/json?poll=1&since=26h"
API = "https://api.github.com/repos/" + os.environ["REPO"]
TOKEN = os.environ["GITHUB_TOKEN"]
LABEL = "crash"
MAX_NEW = 5
MAX_COMMENTS = 20

TITLE_RE = re.compile(r"^rounds-port [0-9A-Za-z.+-]{1,32} crash$")
FP_RE = re.compile(r"^fingerprint: ([0-9a-f]{16})$", re.M)
FIELD_RE = re.compile(r"^(version|os|location|message): (.*)$", re.M)


def gh(method, path, body=None):
    req = urllib.request.Request(
        path if path.startswith("https:") else API + path,
        method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={
            "Authorization": "Bearer " + TOKEN,
            "Accept": "application/vnd.github+json",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read() or b"null")


def reports():
    with urllib.request.urlopen(TOPIC, timeout=30) as r:
        for line in r.read().decode("utf-8", "replace").splitlines():
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if m.get("event") != "message":
                continue
            title, text = m.get("title", ""), m.get("message", "")
            fp = FP_RE.search(text)
            if TITLE_RE.match(title) and fp and len(text) <= 5000:
                yield m["id"], m.get("time", 0), fp.group(1), text


def block(text):
    return "```\n" + text.replace("```", "'''") + "\n```"


def main():
    try:
        gh("POST", "/labels", {"name": LABEL, "color": "b60205"})
    except urllib.error.HTTPError:
        pass  # exists
    issues = gh("GET", f"/issues?labels={LABEL}&state=all&per_page=100")
    by_fp = {}
    for i in issues:
        m = re.search(r"\[([0-9a-f]{16})\]$", i["title"])
        if m:
            by_fp.setdefault(m.group(1), i)
    new = comments = 0
    for msg_id, _, fp, text in sorted(reports(), key=lambda r: r[1]):
        tag = f"<!-- ntfy:{msg_id} -->"
        issue = by_fp.get(fp)
        if issue is None:
            if new >= MAX_NEW:
                continue
            fields = dict(FIELD_RE.findall(text))
            summary = fields.get("message", "panic")[:80]
            body = (
                "Crash report sent by the game (`crates/game/src/crash.rs`).\n\n"
                + block(text)
                + "\n"
                + tag
            )
            issue = gh(
                "POST",
                "/issues",
                {"title": f"Crash: {summary} [{fp}]", "body": body, "labels": [LABEL]},
            )
            by_fp[fp] = issue
            new += 1
            continue
        if tag in (issue.get("body") or ""):
            continue
        seen = gh("GET", issue["comments_url"].replace(API, "") + "?per_page=100")
        if any(tag in (c.get("body") or "") for c in seen):
            continue
        if comments >= MAX_COMMENTS:
            continue
        gh("POST", f"/issues/{issue['number']}/comments", {"body": "Seen again:\n\n" + block(text) + "\n" + tag})
        comments += 1
    print(f"new issues: {new}, comments: {comments}")


if __name__ == "__main__":
    main()
