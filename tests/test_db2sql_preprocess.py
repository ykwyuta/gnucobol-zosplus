import importlib.util
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "db2sql_preprocess", ROOT / "bin" / "db2sql-preprocess.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Db2SqlPreprocessTests(unittest.TestCase):
    def test_demo_rewrites_both_statements(self):
        source = (ROOT / "extras" / "db2sql" / "demo.cbl").read_text()
        output = MODULE.transform(source)
        self.assertIn("COPY 'sqlca.cpy'.", output)
        self.assertEqual(output.count('CALL "DB2SQL_EXEC"'), 6)
        self.assertIn("MOVE '02,00' TO DB2-SQL-META", output)
        self.assertIn("MOVE '01,01' TO DB2-SQL-META", output)
        self.assertTrue(all(len(line) <= 72 for line in output.splitlines()
                            if "DB2-SQL" in line or line.startswith("               '")))

    def test_quotes_do_not_create_host_parameters(self):
        code = MODULE.convert_sql(
            "INSERT INTO T (C, ID) VALUES ('it''s :literal', :HV-ID)", True
        )
        self.assertIn("MOVE '01,00' TO DB2-SQL-META", "".join(code))
        self.assertIn(":literal", "".join(code))

    def test_nullable_hosts_and_qualified_table_declaration(self):
        code = MODULE.convert_sql(
            "SELECT C INTO :HV-OUT :HV-OUT-IND FROM T "
            "WHERE ID = :HV-ID :HV-ID-IND", True
        )
        rendered = "".join(code)
        self.assertIn("MOVE '01,0111' TO DB2-SQL-META", rendered)
        self.assertIn("HV-ID-IND", rendered)
        self.assertIn("HV-OUT-IND", rendered)
        declaration = MODULE.convert_sql(
            "DECLARE STTESTER.CONTROL TABLE (C INTEGER)", True
        )
        self.assertIn("Db2 DECLARE TABLE", "".join(declaration))

    def test_genapp_indicator_cursor_and_identity_assignment(self):
        cursors = {}
        declared = MODULE.convert_sql(
            "DECLARE C INSENSITIVE SCROLL CURSOR FOR "
            "SELECT ID FROM T WHERE K = :KEY INDICATOR :KEY-IND",
            False, cursors,
        )
        self.assertIn("cursor C declared", "".join(declared))
        self.assertEqual(cursors["C"][1], [("KEY", "KEY-IND")])
        held = MODULE.convert_sql(
            "DECLARE H CURSOR WITH HOLD FOR SELECT ID FROM T",
            False, cursors,
        )
        self.assertIn("cursor H declared", "".join(held))
        selected = MODULE.convert_sql(
            "SELECT ID INTO :VALUE INDICATOR :VALUE-IND FROM T "
            "WHERE K = :KEY INDICATOR :KEY-IND", False,
        )
        self.assertIn("MOVE '01,0111' TO DB2-SQL-META", "".join(selected))
        identity = MODULE.convert_sql(
            "SET :NEW-ID = IDENTITY_VAL_LOCAL()", False,
        )
        self.assertIn("SYSDUMMY1", "".join(identity))
        self.assertIn("NEW-ID", "".join(identity))

    def test_unsupported_sql_is_an_error(self):
        with self.assertRaises(MODULE.SqlError):
            MODULE.convert_sql("MERGE INTO T", True)

    def test_copybook_and_cursor(self):
        source = (ROOT / "extras" / "db2sql" /
                  "phase2-demo.cbl").read_text()
        include_dir = ROOT / "extras" / "db2sql"
        output = MODULE.transform(source, include_dirs=(include_dir,))
        self.assertIn("Db2 DECLARE TABLE", output)
        self.assertIn('CALL "DB2SQL_OPEN"', output)
        self.assertEqual(output.count('CALL "DB2SQL_FETCH"'), 3)
        self.assertIn('CALL "DB2SQL_COMMIT"', output)
        self.assertIn('CALL "DB2SQL_ROLLBACK"', output)

    def test_missing_include_is_an_error(self):
        with self.assertRaisesRegex(MODULE.SqlError, "見つかりません"):
            MODULE.transform("       WORKING-STORAGE SECTION.\n"
                             "           EXEC SQL INCLUDE MISSING END-EXEC.\n")

    def test_end_exec_in_literal_does_not_close_block(self):
        self.assertIsNone(MODULE.find_sql_end("VALUES ('END-EXEC')"))
        self.assertEqual(MODULE.find_sql_end("END-EXEC."), 0)

    def test_end_exec_in_block_comment_does_not_close_block(self):
        source = ("       WORKING-STORAGE SECTION.\n"
                  "           EXEC SQL\n"
                  "             /* BEGIN\n"
                  "                END-EXEC */\n"
                  "             DELETE FROM T\n"
                  "           END-EXEC.\n")
        self.assertIn('CALL "DB2SQL_EXEC"', MODULE.transform(source))


if __name__ == "__main__":
    unittest.main()
