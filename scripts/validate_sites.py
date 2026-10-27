"""
Valida o catálogo de sites sem abrir a interface.

Confere se `ui/sites.json` é JSON válido, se cada site tem nome e ícone, se os
arquivos de ícone existem dentro de `ui/` e se o `icon` não aponta para fora do
projeto. Também gera a lista de ids que a interface vai usar.
"""
from __future__ import annotations

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
UI = ROOT / "ui"
CATALOG = UI / "sites.json"

ALLOWED = re.compile(r"^[A-Za-z0-9._/-]+$")


def normalize(path: str) -> str | None:
    value = str(path or "").strip().replace("\\", "/")
    while value.startswith("./"):
        value = value[2:]
    if not value or value.startswith("/") or ".." in value or "//" in value:
        return None
    if not ALLOWED.match(value) or not value.startswith("assets/"):
        return None
    return value


def slug(name: str) -> str:
    base = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return (base or "site")[:80]


def main() -> int:
    if not CATALOG.is_file():
        print(f"ERRO: {CATALOG} não encontrado", file=sys.stderr)
        return 1
    try:
        data = json.loads(CATALOG.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        print(f"ERRO: sites.json inválido: {exc}", file=sys.stderr)
        return 1
    if not isinstance(data, dict) or not data:
        print("ERRO: sites.json deve ser um objeto com pelo menos um site", file=sys.stderr)
        return 1

    failures = 0
    seen: set[str] = set()
    for name, meta in data.items():
        name = str(name).strip()
        if not name or len(name) > 100:
            print(f"ERRO: nome de site inválido: {name!r}", file=sys.stderr)
            failures += 1
            continue
        icon = (meta or {}).get("icon") if isinstance(meta, dict) else meta
        text_icon = isinstance(icon, str) and 0 < len(icon.strip()) <= 2 and "/" not in icon and "\\" not in icon
        if text_icon:
            print(f"OK   {name:16} id={slug(name):16} ícone=texto({icon.strip()})")
            continue
        relative = normalize(str(icon or ""))
        if relative is None:
            print(f"ERRO {name:16} icon inválido: {icon!r}", file=sys.stderr)
            failures += 1
            continue
        target = (UI / relative).resolve()
        try:
            target.relative_to(UI.resolve())
        except ValueError:
            print(f"ERRO {name:16} icon fora de ui/: {relative}", file=sys.stderr)
            failures += 1
            continue
        if not target.is_file():
            print(f"ERRO {name:16} arquivo ausente: {relative}", file=sys.stderr)
            failures += 1
            continue
        print(f"OK   {name:16} id={slug(name):16} ícone={relative} ({target.stat().st_size} bytes)")
    print(f"{len(data)} site(s), {failures} problema(s).")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
