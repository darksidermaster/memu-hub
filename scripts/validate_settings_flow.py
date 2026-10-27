"""
Valida o fluxo "cadastrar site do catálogo + duas API-keys" contra a MESMA
validação Rust usada pelo `save_settings` do aplicativo.

O teste de interface usa um backend simulado, então ele não pegaria uma regra
de validação errada como a que recusava o ícone do catálogo
(`assets/logos/x.svg`, mais de 2 caracteres). Este script chama o binário em
modo CLI `validate-settings` para exercitar o código Rust real.
"""
from __future__ import annotations

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
EXE = ROOT / "src-tauri" / "target" / "debug" / ("memu-hub.exe" if sys.platform == "win32" else "memu-hub")


def settings(sites, keys):
    priority = []
    for key in keys:
        if key["site_id"] not in priority:
            priority.append(key["site_id"])
    return {
        "memuc_path": "",
        "adb_path": "",
        "package_name": "com.away.game.br",
        "demo_emulators": False,
        "sites": sites,
        "keys": keys,
        "priority": priority,
        "instances": {},
    }


def run(payload):
    result = subprocess.run(
        [str(EXE), "validate-settings"],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=60,
    )
    return result.returncode, (result.stdout + result.stderr).strip()


def main() -> int:
    if not EXE.is_file():
        print(f"ERRO: binário não encontrado em {EXE}. Compile com cargo build.", file=sys.stderr)
        return 2

    capsonic = {"id": "capsonic", "name": "CapSonic", "icon": "assets/logos/capsonic.svg"}
    nonecap = {"id": "nonecap", "name": "NoneCap", "icon": "assets/logos/nonecap.webp"}
    key1 = {"id": "k1", "site_id": "capsonic", "label": "Conta principal", "secret": "x"}
    key2 = {"id": "k2", "site_id": "capsonic", "label": "Conta 2", "secret": "y"}

    cases = [
        ("catálogo com ícone de arquivo", settings([capsonic], []), True),
        ("duas chaves no mesmo site", settings([capsonic], [key1, key2]), True),
        ("dois sites do catálogo", settings([capsonic, nonecap], [key1]), True),
        ("ícone textual legado", settings([{"id": "a", "name": "A", "icon": "A"}], []), True),
        ("ícone inválido (>2 chars)", settings([{"id": "a", "name": "A", "icon": "ABC"}], []), False),
        ("caminho fora de assets", settings([{"id": "a", "name": "A", "icon": "../x.png"}], []), False),
        ("site duplicado", settings([capsonic, dict(capsonic)], []), False),
    ]

    failures = 0
    for label, payload, expected_ok in cases:
        code, output = run(payload)
        ok = code == 0
        status = "OK  " if ok == expected_ok else "FALHA"
        if ok != expected_ok:
            failures += 1
        print(f"{status} {label}: exit={code} {output[:160]}")
    print(f"{len(cases)} caso(s), {failures} problema(s).")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
