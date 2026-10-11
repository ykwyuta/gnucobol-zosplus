import importlib.util
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[3] / "bin/imsdli-preprocess.py"
spec = importlib.util.spec_from_file_location("execdli", SCRIPT)
execdli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(execdli)


class ExecDliTest(unittest.TestCase):
    def test_get_uses_hold_and_preserves_qualified_value(self):
        source = (
            "       WORKING-STORAGE SECTION.\n"
            "       LINKAGE SECTION.\n"
            "           EXEC DLI GU USING PCB(PAUT-PCB-NUM)\n"
            "               SEGMENT (PAUTSUM0)\n"
            "               INTO (PENDING-AUTH-SUMMARY)\n"
            "               WHERE (ACCNTID = PA-ACCT-ID)\n"
            "           END-EXEC\n"
        )
        result = execdli.translate(source, "DBPAUTP0")
        self.assertIn("MOVE 'GHU ' TO IMS-DLI-FUNCTION", result)
        self.assertIn("'ACCNTID ' 'EQ'", result)
        self.assertIn("CALL STATIC 'IMSDSA' USING IMS-DLI-SSA-1",
                      result)
        self.assertIn("IMS-DLI-SSA-1 PA-ACCT-ID", result)
        self.assertIn("MOVE IMS-DLI-STATUS TO DIBSTAT", result)

    def test_two_segment_insert_and_service_calls(self):
        source = (
            "       LINKAGE SECTION.\n"
            "           EXEC DLI ISRT USING PCB(PAUT-PCB-NUM)\n"
            "               SEGMENT (PAUTSUM0)\n"
            "               WHERE (ACCNTID = PA-ACCT-ID)\n"
            "               SEGMENT (PAUTDTL1)\n"
            "               FROM (PENDING-AUTH-DETAILS)\n"
            "               SEGLENGTH (LENGTH OF PENDING-AUTH-DETAILS)\n"
            "           END-EXEC\n"
            "           EXEC DLI CHKP ID(WK-CHKPT-ID) END-EXEC\n"
            "           EXEC DLI SCHD PSB((PSB-NAME)) NODHABEND\n"
            "           END-EXEC\n"
            "           EXEC DLI TERM END-EXEC\n"
        )
        result = execdli.translate(source, "DBPAUTP0")
        self.assertIn("IMS-DLI-SSA-1 IMS-DLI-SSA-2", result)
        self.assertIn("MOVE WK-CHKPT-ID TO IMS-DLI-ARG", result)
        self.assertIn("MOVE PSB-NAME TO IMS-DLI-ARG", result)
        self.assertIn("MOVE 'TERM' TO IMS-DLI-FUNCTION", result)

    def test_unknown_clause_fails_with_line(self):
        source = (
            "       LINKAGE SECTION.\n"
            "           EXEC DLI GU USING PCB(PAUT-PCB-NUM)\n"
            "               SEGMENT (PAUTSUM0) INTO (DATA) BOGUS(1)\n"
            "           END-EXEC\n"
        )
        with self.assertRaisesRegex(ValueError, "line 2: unsupported EXEC DLI clause BOGUS"):
            execdli.translate(source, "DBPAUTP0")

    def test_cics_terminal_program_without_linkage_section(self):
        source = (
            "       WORKING-STORAGE SECTION.\n"
            "       PROCEDURE DIVISION.\n"
            "           EXEC DLI SCHD PSB((PSB-NAME)) END-EXEC\n"
        )
        result = execdli.translate(source, "DBPAUTP0")
        self.assertLess(result.index("01 DIBSTAT"),
                        result.index("PROCEDURE DIVISION"))
        self.assertIn("MOVE 'SCHD' TO IMS-DLI-FUNCTION", result)


if __name__ == "__main__":
    unittest.main()
