#!/usr/bin/env python3
"""Keep production query construction in database; inspect cfg(test) modules precisely."""
import re
import sys
from pathlib import Path

# Mask literals/comments so SQL strings and braces in fixtures are not Rust syntax.
LITERALS = re.compile(
    r'//[^\n]*|/\*.*?\*/|r(?P<hashes>\#*)".*?"(?P=hashes)|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'',
    re.S,
)
TEST_MODULE = re.compile(r'#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*(?:#\s*\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*([;{])')
QUERY_NAME = r'(?:query(?:_as|_scalar)?(?:_with)?|raw_sql)'
QUERY = re.compile(r'\bQueryBuilder\b|(?<![.\w])' + QUERY_NAME + r'\b(?=\s*(?:::\s*<[^;]*?>)?\s*[(!])')
SQLX_IMPORT = re.compile(r'\buse\s+sqlx\s*::[^;]*;')
MODULE = re.compile(r'(?:#\s*\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;')


def module_paths(path, name, attributes):
    attr = re.search(r'#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]', attributes)
    if attr:
        return {(path.parent / attr[1]).resolve()}
    module_dir = path.parent if path.name in ('mod.rs', 'lib.rs', 'main.rs') else path.with_suffix('')
    return {(module_dir / (name + '.rs')).resolve(), (module_dir / name / 'mod.rs').resolve()}


def mask_literals(source):
    return LITERALS.sub(lambda m: re.sub(r'[^\n]', ' ', m.group()), source)


def production_source(source, path):
    """Remove only explicit test module bodies; collect their external source paths."""
    masked = mask_literals(source)
    external = set()
    for match in reversed(list(TEST_MODULE.finditer(masked))):
        start, end = match.span()
        if match[2] == '{':
            depth = 1
            while depth and end < len(masked):
                depth += (masked[end] == '{') - (masked[end] == '}')
                end += 1
            if depth:
                raise ValueError(f'{path}: unclosed test module')
        else:
            external.update(module_paths(path, match[1], source[start:end]))
        masked = masked[:start] + re.sub(r'[^\n]', ' ', masked[start:end]) + masked[end:]
    return masked, external


def violations(root):
    paths = sorted(root.rglob('*.rs'))
    production = {}
    test_only = set()
    for path in paths:
        production[path], external = production_source(path.read_text(), path)
        test_only.update(external)
    # Children of explicitly test-only external modules are also fixtures.
    for path in list(test_only):
        module_dir = path.parent if path.name == 'mod.rs' else path.with_suffix('')
        if module_dir.is_dir():
            test_only.update(child.resolve() for child in module_dir.rglob('*.rs'))
    # A source shared by a production module and a test module remains production.
    imports = {}
    for path, source in production.items():
        raw = path.read_text()
        imports[path.resolve()] = set()
        for match in MODULE.finditer(source):
            imports[path.resolve()].update(module_paths(path, match[1], raw[match.start():match.end()]))
    reachable = {path.resolve() for path in paths} - test_only
    frontier = list(reachable)
    while frontier:
        for child in imports.get(frontier.pop(), set()):
            if child not in reachable:
                reachable.add(child)
                frontier.append(child)
    test_only -= reachable
    found = []
    for path, source in production.items():
        if 'database' == path.relative_to(root).parts[0] or path.resolve() in test_only:
            continue
        matches = list(QUERY.finditer(source))
        for imported in SQLX_IMPORT.finditer(source):
            for name in re.finditer(r'\b' + QUERY_NAME + r'\b', imported[0]):
                start = imported.start() + name.start()
                line = source.count('\n', 0, start) + 1
                found.append(f'{path}:{line}: query import belongs under database/')
        for match in matches:
            # Catch constructors, imported functions/macros, QueryBuilder and renamed imports.
            line = source.count('\n', 0, match.start()) + 1
            found.append(f'{path}:{line}: {match[0]} belongs under database/')
    return found


if __name__ == '__main__':
    root = Path(__file__).resolve().parents[1] / 'crates/voice-agent-server/src'
    errors = violations(root)
    print('\n'.join(errors) if errors else 'PASS: production SQLx queries belong to database/')
    sys.exit(bool(errors))
