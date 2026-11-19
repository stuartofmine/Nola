#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Export a reviewed current source tree without private Git history or local keys."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

RULES = {
    'private_key': re.compile(r'-----BEGIN (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----\s+[A-Za-z0-9+/=\r\n]{40,}-----END (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----'),
    'github_token': re.compile(r'\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})\b'),
    'provider_key': re.compile(r'\b(?:sk-(?:proj-|ant-)?[A-Za-z0-9_-]{30,}|AIza[A-Za-z0-9_-]{30,}|AKIA[A-Z0-9]{16})\b'),
    'secret_assignment': re.compile(r'''(?i)(?:api[_-]?key|client[_-]?secret|secret[_-]?key|access[_-]?token|password)\s*[=:]\s*["']([A-Za-z0-9_+/=-]{24,})["']'''),
}
SECRET_EXTENSIONS={'.p8','.p12','.pfx','.jks','.keystore','.mobileprovision','.pem','.db','.sqlite','.sqlite3','.profraw','.profdata'}
PRIVATE_PARTS={'.git','.github','.vscode','.claude','.cargo','.codex','.idea','.nola','.nola-host-secrets','node_modules','target','build','DerivedData','.artifacts','.wrangler','xcuserdata','__pycache__','.pytest_cache','.gradle','.cxx','.kotlin'}


def excluded(name):
    path=Path(name)
    return (path.is_absolute() or '..' in path.parts or path.parts[0] in {'cer','Doc','legacy','backend'} or
            bool(set(path.parts)&PRIVATE_PARTS) or any(part.endswith(('.xcresult','.dSYM')) for part in path.parts) or path.suffix.lower() in SECRET_EXTENSIONS or
            path.name in {'.DS_Store','.env','.dev.vars','.netrc','.npmrc','local.properties'} or
            (path.name.startswith('.env.') and not path.name.endswith('.example')) or path.name.endswith(('.log','.db-wal','.db-shm')))


def findings(path, text):
    for rule,pattern in RULES.items():
        for match in pattern.finditer(text):
            value=match.group(1) if rule=='secret_assignment' else match.group()
            yield {'path':path,'rule':rule,'sha256':hashlib.sha256(value.encode()).hexdigest(),
                   'line':text.count('\n',0,match.start())+1}


def export(root, destination, allowlist=None, revision="HEAD"):
    if destination.exists():
        raise ValueError("Export destination must not exist")
    commit = subprocess.check_output(
        ["git", "rev-parse", "--verify", "--end-of-options", revision + "^{commit}"],
        cwd=root, text=True,
    ).strip()
    with tempfile.TemporaryDirectory(prefix="nola-public-source-") as temporary:
        snapshot = Path(temporary)
        with tempfile.TemporaryFile() as stream:
            subprocess.run(["git", "archive", "--format=tar", commit], cwd=root, stdout=stream, check=True)
            stream.seek(0)
            omitted = []
            names = []
            with tarfile.open(fileobj=stream, mode="r:") as archive:
                for member in archive:
                    name = member.name
                    if member.isdir():
                        continue
                    if excluded(name):
                        omitted.append(name)
                        continue
                    target = snapshot / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    if member.issym():
                        target.symlink_to(member.linkname)
                    elif member.isfile():
                        if member.size >= 100 * 1024 * 1024:
                            raise ValueError("GitHub file limit exceeded: " + name)
                        with archive.extractfile(member) as source, target.open("wb") as output:
                            shutil.copyfileobj(source, output)
                        target.chmod(member.mode & 0o777)
                    else:
                        raise ValueError("Unsupported archive member: " + name)
                    names.append(name)
        if allowlist is None:
            manifest = snapshot / "release/public-fixtures.json"
            if "release/public-fixtures.json" not in names or manifest.is_symlink():
                raise ValueError("Public fixture allowlist must be a regular file")
            allowlist = json.loads(manifest.read_text())
        report = copy_reviewed_files(snapshot, destination, allowlist, names, omitted)
        return {**report, "commit": commit}


def copy_reviewed_files(root, destination, allowlist, names, omitted):
    root = root.resolve()
    allowed={(item['path'],item['rule'],item['sha256']) for item in allowlist}
    plan=[];blocked=[]
    for name in names:
        if not name:continue
        path=root/name
        if not path.exists() and not path.is_symlink():continue
        if path.is_symlink():
            resolved = path.resolve()
            if (path.readlink().is_absolute() or resolved == root or not resolved.is_relative_to(root)
                    or excluded(str(resolved.relative_to(root)))):
                raise ValueError('Export symlink escapes the public source: '+name)
            plan.append(name);continue
        if not path.is_file():continue
        try:text=path.read_text()
        except UnicodeError:text=None
        if text is not None:
            for item in findings(name,text):
                if (item['path'],item['rule'],item['sha256']) not in allowed:blocked.append(item)
        plan.append(name)
    if blocked:
        # Report locations and fingerprints only; never print credential values.
        raise ValueError('Unreviewed credential findings: '+json.dumps(blocked))
    destination.mkdir(parents=True)
    for name in plan:
        source=root/name;target=destination/name;target.parent.mkdir(parents=True,exist_ok=True)
        if source.is_symlink():target.symlink_to(source.readlink())
        else:shutil.copy2(source,target)
    return {'files':len(plan),'excluded_paths':omitted,'history_copied':False}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--destination',required=True,type=Path)
    parser.add_argument('--commit', default='HEAD', help='Reviewed commit to export (default: HEAD)')
    args=parser.parse_args();root=Path(__file__).resolve().parents[2]
    try:
        value=export(root,args.destination.resolve(),revision=args.commit)
        print(json.dumps(value,indent=2))
    except (ValueError,OSError,subprocess.CalledProcessError) as error:parser.exit(1,f'Public export stopped: {error}\n')


if __name__=='__main__':main()
