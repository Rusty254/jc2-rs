#!/usr/bin/env python3
"""Build the public, executable-free snapshot of jc2-rs and commit it on the `public` branch.

The full repository (with research derived from JustCause2.exe) stays local. The public
GitHub repo only ever receives commits made by this script on the orphan `public` branch:

  python scripts/publish_public.py            # export master -> commit on `public`
  python scripts/publish_public.py --check    # export to a temp dir and report, no commit
  git push github public:main                 # publish (after reviewing the commit)

Excluded entirely: research/analysis/** (extracts of the executable), research/tools/**
except our own generic tools, research/ghidra/**, notes/task-board*.json. In every other
text file, fenced code blocks and lines that quote decompiler output (Ghidra-style tokens)
are removed; prose, file-format descriptions and native addresses stay.
"""
import os, re, shutil, subprocess, sys, tempfile

ROOT = subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip()
EXCLUDE_DIRS = ("research/analysis/", "research/ghidra/", "research/tools/")
KEEP_TOOLS = {"research/tools/jenkins.py", "research/tools/render_obj.py",
              "research/tools/ghidra_scripts/ExportDecompProject.java"}
EXCLUDE_FILES = re.compile(r"^notes/task-board.*\.json$")
TOKENS = re.compile(r"\bundefined[1-8]?\b|\b(?:uVar|iVar|fVar|pcVar|puVar|piVar|lVar|bVar|cVar|sVar|local_|param_|in_stack_|extraout_|unaff_|pfVar|ppuVar)\w*|\(\*\*?\(code \*\*?\)|\bFUN_[0-9a-f]{8}\s*\(")
TEXT_EXT = (".md", ".txt", ".rs", ".wgsl", ".toml", ".py", ".ps1", ".sh", ".json", ".csv", ".java")
NOTE = ("\n> Public snapshot: research extracted from JustCause2.exe (function/string lists, "
        "decompiler excerpts and probes) is kept out of this repository. Native addresses cited "
        "in the documents refer to the retail Steam executable.\n")

def included(path):
    if path.startswith(EXCLUDE_DIRS) and path not in KEEP_TOOLS:
        return False
    return not EXCLUDE_FILES.match(path)

def sanitize(text):
    out, block, in_block, dropped = [], [], False, 0
    for line in text.splitlines(keepends=True):
        if line.lstrip().startswith("```"):
            if in_block:
                block.append(line)
                if any(TOKENS.search(l) for l in block):
                    dropped += 1
                    out.append("_(decompiler excerpt omitted from the public snapshot)_\n")
                else:
                    out.extend(block)
                block, in_block = [], False
            else:
                in_block, block = True, [line]
            continue
        if in_block:
            block.append(line)
        elif TOKENS.search(line):
            dropped += 1
        else:
            out.append(line)
    out.extend(block)
    return "".join(out), dropped

def export(dest):
    files = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", "master"], cwd=ROOT, text=True).split("\n")
    kept = skipped = edited = 0
    for path in filter(None, files):
        if not included(path):
            skipped += 1
            continue
        data = subprocess.check_output(["git", "show", f"master:{path}"], cwd=ROOT)
        target = os.path.join(dest, path)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        if path.endswith(TEXT_EXT):
            text = data.decode("utf-8", "replace")
            # Rust sources keep decompiler-style identifiers out by policy; only docs/tools are scrubbed
            if not path.endswith((".rs", ".wgsl", ".toml")) and not path.startswith("scripts/"):
                text, n = sanitize(text)
                edited += bool(n)
            if path == "README.md":
                text += NOTE
            data = text.encode("utf-8")
        with open(target, "wb") as f:
            f.write(data)
        kept += 1
    leftover = [os.path.relpath(os.path.join(d, f), dest) for d, _, fs in os.walk(dest) for f in fs
                if f.endswith(TEXT_EXT) and not f.endswith((".rs", ".wgsl", ".toml"))
                and not os.path.relpath(d, dest).replace(os.sep, "/").startswith("scripts")
                and any(TOKENS.search(l) for l in open(os.path.join(d, f), encoding="utf-8", errors="replace"))]
    return kept, skipped, edited, leftover

def main():
    check = "--check" in sys.argv
    tmp = tempfile.mkdtemp(prefix="jc2-public-")
    kept, skipped, edited, leftover = export(tmp)
    print(f"kept {kept} files, excluded {skipped}, scrubbed {edited}; leftover decompiler tokens in: {leftover or 'none'}")
    if leftover:
        sys.exit("refusing to publish: decompiler tokens remain")
    if check:
        print(f"snapshot at {tmp}")
        return
    wt = tempfile.mkdtemp(prefix="jc2-public-wt-")
    shutil.rmtree(wt)
    has = subprocess.run(["git", "rev-parse", "-q", "--verify", "public"], cwd=ROOT, capture_output=True).returncode == 0
    if has:
        subprocess.check_call(["git", "worktree", "add", wt, "public"], cwd=ROOT)
    else:
        subprocess.check_call(["git", "worktree", "add", "--detach", wt], cwd=ROOT)
        subprocess.check_call(["git", "checkout", "--orphan", "public"], cwd=wt)
    subprocess.check_call(["git", "rm", "-rq", "--cached", "--ignore-unmatch", "."], cwd=wt)
    for entry in os.listdir(wt):
        if entry != ".git":
            p = os.path.join(wt, entry)
            shutil.rmtree(p) if os.path.isdir(p) else os.remove(p)
    shutil.copytree(tmp, wt, dirs_exist_ok=True)
    subprocess.check_call(["git", "add", "-A"], cwd=wt)
    head = subprocess.check_output(["git", "rev-parse", "--short", "master"], cwd=ROOT, text=True).strip()
    r = subprocess.run(["git", "commit", "-qm", f"Public snapshot of master {head}\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"], cwd=wt)
    print("committed" if r.returncode == 0 else "nothing to commit")
    subprocess.check_call(["git", "worktree", "remove", "--force", wt], cwd=ROOT)
    shutil.rmtree(tmp, ignore_errors=True)

if __name__ == "__main__":
    main()
