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
    return (path.is_absolute() or '..' in path.parts or path.parts[0]=='cer' or
            bool(set(path.parts)&PRIVATE_PARTS) or any(part.endswith(('.xcresult','.dSYM')) for part in path.parts) or path.suffix.lower() in SECRET_EXTENSIONS or
            path.name in {'.DS_Store','.env','.dev.vars','.netrc','.npmrc','local.properties'} or
            (path.name.startswith('.env.') and not path.name.endswith('.example')) or path.name.endswith(('.log','.db-wal','.db-shm')))


def findings(path, text):
    for rule,pattern in RULES.items():
        for match in pattern.finditer(text):
            value=match.group(1) if rule=='secret_assignment' else match.group()
            yield {'path':path,'rule':rule,'sha256':hashlib.sha256(value.encode()).hexdigest(),
                   'line':text.count('\n',0,match.start())+1}


def export(root, destination, allowlist):
    if destination.exists():raise ValueError('Export destination must not exist')
    allowed={(item['path'],item['rule'],item['sha256']) for item in allowlist}
    names=sorted(set(subprocess.check_output(['git','ls-files','-c','-o','--exclude-standard','-z'],cwd=root).decode().split('\0')))
    plan=[];omitted=[];blocked=[]
    for name in names:
        if not name:continue
        path=root/name
        if excluded(name):omitted.append(name);continue
        if not path.exists() and not path.is_symlink():continue
        if path.is_symlink():
            if not path.resolve().is_relative_to(root.resolve()) or excluded(str(path.resolve().relative_to(root.resolve()))):
                raise ValueError('Export symlink escapes the public source: '+name)
            plan.append(name);continue
        if not path.is_file():continue
        if path.stat().st_size>=100*1024*1024:raise ValueError('GitHub file limit exceeded: '+name)
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
    args=parser.parse_args();root=Path(__file__).resolve().parents[2]
    try:
        allowlist=json.loads((root/'release/public-fixtures.json').read_text())
        value=export(root,args.destination.resolve(),allowlist)
        print(json.dumps(value,indent=2))
    except (ValueError,OSError) as error:parser.exit(1,f'Public export stopped: {error}\n')


if __name__=='__main__':main()
