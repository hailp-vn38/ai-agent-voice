import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('placement', Path(__file__).parents[1] / 'check-sqlx-placement.py')
placement = importlib.util.module_from_spec(spec)
spec.loader.exec_module(placement)


class PlacementTest(unittest.TestCase):
    def test_test_module_does_not_exempt_later_production_code(self):
        source = '''#[cfg(test)] mod tests { fn fixture() { sqlx::query("}"); } }
fn production() { sqlx::query("SELECT 1"); }
'''
        production, _ = placement.production_source(source, Path('/tmp/example.rs'))
        self.assertEqual(len(list(placement.QUERY.finditer(production))), 1)

    def test_literals_and_comments_are_not_code(self):
        source = '''// sqlx::query("SELECT 1")
const DOC: &str = r#"sqlx::query("{")"#;
/* QueryBuilder */ fn real() { sqlx::query_as::<_, ()>("SELECT 1"); }
'''
        production, _ = placement.production_source(source, Path('/tmp/example.rs'))
        self.assertEqual([m[0] for m in placement.QUERY.finditer(production)], ['query_as'])

    def test_external_test_module_and_renamed_query_import(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'database').mkdir()
            (root / 'database/mod.rs').write_text('sqlx::query("SELECT 1");')
            (root / 'lib.rs').write_text('''#[cfg(test)] #[path="fixtures.rs"] mod fixtures;
use sqlx::{query as run_sql, QueryBuilder};
''')
            (root / 'fixtures.rs').write_text('sqlx::query("SELECT 1");')
            violations = placement.violations(root)
            self.assertEqual(len(violations), 2)
            self.assertTrue(all('lib.rs' in violation for violation in violations))

    def test_shared_test_source_is_still_checked_when_used_in_production(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'lib.rs').write_text('mod shared; #[cfg(test)] #[path="shared.rs"] mod test_shared;')
            (root / 'shared.rs').write_text('fn read() { sqlx::query("SELECT 1"); }')
            self.assertEqual(len(placement.violations(root)), 1)

    def test_query_with_constructors_and_renamed_imports(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ('query_with', 'query_as_with', 'query_scalar_with'):
                with self.subTest(name=name):
                    (root / 'lib.rs').write_text(f'fn read() {{ sqlx::{name}("SELECT 1", args); }}')
                    self.assertEqual(len(placement.violations(root)), 1)
                    (root / 'lib.rs').write_text(f'use sqlx::{name} as run_sql;')
                    self.assertEqual(len(placement.violations(root)), 1)


if __name__ == '__main__':
    unittest.main()
