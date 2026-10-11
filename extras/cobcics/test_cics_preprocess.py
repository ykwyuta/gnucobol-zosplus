import importlib.util
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "cics_preprocess", ROOT / "bin" / "cics-preprocess.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CicsPreprocessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # A small generated-copybook excerpt is enough to exercise map lookup.
        cls.copybook = ("       01 BNK1MEI.\n"
                        "       02 ACTIONL PIC S9(4) COMP.\n"
                        "       02 ACTIONF PIC X.\n"
                        "       02 ACTIONI PIC X(1).\n"
                        "       01 BNK1MEO REDEFINES BNK1MEI.\n"
                        "       02 ACTIONO PIC X(1).\n")
        cls.maps = MODULE.maps_from_copybook(cls.copybook)

    def test_demo_translates_cics_to_static_calls(self):
        source = (ROOT / "extras" / "cobcics" /
                  "menu-worker-cics.cob").read_text()
        output = MODULE.transform(source, self.maps)
        self.assertIn('CALL STATIC "COBCICS_START"', output)
        self.assertIn('CALL STATIC "COBCICS_GET_FIELD"', output)
        self.assertIn('CALL STATIC "COBCICS_RECEIVE_STATUS"', output)
        self.assertIn('CALL STATIC "COBCICS_SEND_FIELD"', output)
        self.assertIn('CALL STATIC "COBCICS_SEND_MAP"', output)
        self.assertIn('CALL STATIC "COBCICS_SEND_CONTROL"', output)
        self.assertIn('CALL STATIC "COBCICS_SEND_TEXT"', output)
        self.assertIn('CALL STATIC "COBCICS_ASSIGN_APPLID"', output)
        self.assertIn('CALL STATIC "COBCICS_ASSIGN_PROGRAM"', output)
        self.assertIn('"MENUCICS"', output)
        self.assertIn('CALL STATIC "COBCICS_LINK_ENTER"', output)
        self.assertIn('CALL STATIC "COBCICS_LINK_EXIT"', output)
        self.assertIn('CALL STATIC "COBCICS_GET_COMMAREA"', output)
        self.assertIn('CALL STATIC "COBCICS_SET_RETURN"', output)
        self.assertIn('USING "OMEN" COMMUNICATION-AREA', output)
        self.assertIn('MOVE EIBRESP TO WS-CICS-RESP', output)
        self.assertIn('WS-CICS-RESP = 36', output)
        self.assertIn('CALL STATIC "COBCICS_RETURN"', output)
        self.assertNotIn("EXEC CICS", output)
        self.assertTrue(all(len(line) <= 72 for line in output.splitlines()
                            if "COBCICS" in line or "USING" in line))

    def test_unsupported_command_and_option_have_line(self):
        prefix = ("       DATA DIVISION.\n"
                  "       WORKING-STORAGE SECTION.\n"
                  "       PROCEDURE DIVISION.\n")
        for command, expected in [
            ("EXEC CICS XCTL PROGRAM('OTHER') END-EXEC.", "XCTL"),
            ("EXEC CICS SEND MAP('BNK1ME') MAPONLY DATAONLY END-EXEC.",
             "MAPONLY and DATAONLY"),
        ]:
            with self.subTest(command=command):
                with self.assertRaisesRegex(MODULE.CicsError, f"4: .*{expected}"):
                    MODULE.transform(prefix + "           " + command + "\n", self.maps)
        with self.assertRaisesRegex(MODULE.CicsError, "MAPSET does not match"):
            MODULE.transform(prefix +
                             "           EXEC CICS SEND MAP('BNK1ME') "
                             "MAPSET('OTHER') END-EXEC.\n",
                             self.maps, expected_mapset="BNK1MAI")

    def test_mq_trigger_retrieve_and_carddemo_dataset_read(self):
        retrieved = "".join(MODULE.statement(
            "RETRIEVE INTO(MQTM) NOHANDLE", self.maps, False,
            None, None, None))
        self.assertIn('CALL STATIC "COBCICS_RETRIEVE"', retrieved)
        self.assertNotIn('COBCICS_REQUIRE_NORMAL', retrieved)
        read = "".join(MODULE.statement(
            "READ DATASET(WS-FILE) INTO(ACCT-ROW) "
            "LENGTH(LENGTH OF ACCT-ROW) RIDFLD(ACCT-ID) "
            "KEYLENGTH(LENGTH OF ACCT-ID) RESP(WS-RESP)",
            self.maps, False, None, None, None))
        self.assertIn("MOVE WS-FILE", read)
        self.assertIn('CALL STATIC "COBCICS_FILE_READ"', read)

    def test_xctl_passes_commarea_and_does_not_resume_caller(self):
        translated = "".join(MODULE.statement(
            "XCTL PROGRAM(NEXT-PROGRAM) COMMAREA(DFHCOMMAREA)",
            self.maps, False, None, "CURRENT", None))
        self.assertIn("CALL NEXT-PROGRAM USING DFHCOMMAREA", translated)
        self.assertIn('CALL STATIC "COBCICS_LINK_EXIT"', translated)
        self.assertIn('CALL STATIC "COBCICS_RETURN"', translated)
        self.assertTrue(translated.rstrip().endswith("GOBACK"))
        self.assertNotIn("COBCICS_REQUIRE_NORMAL", translated)
        with self.assertRaisesRegex(MODULE.CicsError,
                                    "unsupported XCTL option: SYNCONRETURN"):
            MODULE.statement("XCTL PROGRAM('NEXT') COMMAREA(DATA) "
                             "SYNCONRETURN", self.maps, False)

    def test_carddemo_ims_cics_options(self):
        received = "".join(MODULE.statement(
            "RECEIVE MAP('BNK1ME') INTO(BNK1MEI) NOHANDLE",
            self.maps, False))
        self.assertIn("COBCICS_RECEIVE_STATUS", received)
        self.assertNotIn("COBCICS_REQUIRE_NORMAL", received)
        timestamp = "".join(MODULE.statement(
            "FORMATTIME ABSTIME(NOW-TIME) YYDDD(DAY) "
            "TIME(CLOCK) MILLISECONDS(MILLIS)", self.maps, False))
        self.assertIn("COBCICS_FORMATTIME_YYDDD", timestamp)
        self.assertIn("MILLIS", timestamp)
        queue = "".join(MODULE.statement(
            "WRITEQ TD QUEUE('CSSL') FROM(MSG) NOHANDLE",
            self.maps, False))
        self.assertIn('MOVE "CSSL    " TO COBCICS-QUEUE-NAME', queue)
        self.assertNotIn("COBCICS_REQUIRE_NORMAL", queue)

    def test_fixed_columns_preserved_when_rewriting_dfhresp(self):
        line = ("000100     IF EIBRESP = DFHRESP(NORMAL)".ljust(72)
                + "12345678\n")
        output = MODULE.rewrite_dfhresp(line, 1)
        self.assertEqual(output[72:80], "12345678")
        self.assertIn("EIBRESP = 0", output[7:72])

    def test_quotes_comments_and_missing_end(self):
        self.assertIsNone(MODULE.find_end("MAP('END-EXEC')"))
        self.assertEqual(MODULE.find_end("MAP('A') END-EXEC."), 9)
        prefix = ("       DATA DIVISION.\n"
                  "       WORKING-STORAGE SECTION.\n"
                  "       PROCEDURE DIVISION.\n")
        output = MODULE.transform(prefix + "      * EXEC CICS LINK END-EXEC.\n"
                                  "      D    EXEC CICS LINK END-EXEC.\n"
                                  "           EXEC CICS RETURN END-EXEC.\n", self.maps)
        self.assertIn("* EXEC CICS LINK", output)
        self.assertIn("D    EXEC CICS LINK", output)
        with self.assertRaisesRegex(MODULE.CicsError, "missing END-EXEC"):
            MODULE.transform(prefix + "           EXEC CICS RETURN\n", self.maps)

    def test_occurs_map_uses_indexed_fields(self):
        copybook = self.copybook + (
            "       02 ACCOUNTG OCCURS 10.\n"
            "       03 ACCOUNTO PIC X(79).\n")
        maps = MODULE.maps_from_copybook(copybook)
        self.assertIn(("ACCOUNT", 10), maps["BNK1ME"])
        lines = MODULE.statement("SEND MAP('BNK1ME')", maps, False, None, "BNK1ME", None)
        self.assertIn('USING "ACCO0001"', "".join(lines))
        self.assertIn("ACCOUNTO(10)", "".join(lines))

    def test_extended_symbolic_attributes_reach_bridge(self):
        copybook = self.copybook + (
            "       02 ACTIONC PIC X.\n"
            "       02 ACTIONH PIC X.\n")
        maps = MODULE.maps_from_copybook(copybook)
        lines = "".join(MODULE.statement(
            "SEND MAP('BNK1ME')", maps, False, None, "BNK1ME", None))
        self.assertEqual(lines.count('CALL STATIC "COBCICS_SEND_EXT_ATTRIBUTE"'), 2)
        self.assertIn("ACTIONC", lines)
        self.assertIn("ACTIONH", lines)
        self.assertTrue(all(len(line) <= 72 for line in lines.splitlines()))

    def test_dfhresp_rewrite_skips_literals_and_rejects_unknown(self):
        prefix = ("       DATA DIVISION.\n"
                  "       WORKING-STORAGE SECTION.\n"
                  "       PROCEDURE DIVISION.\n")
        output = MODULE.transform(prefix +
                                  "           DISPLAY 'DFHRESP(MAPFAIL)'\n"
                                  "           IF EIBRESP = DFHRESP(NORMAL)\n"
                                  "               CONTINUE\n"
                                  "           END-IF.\n", self.maps)
        self.assertIn("'DFHRESP(MAPFAIL)'", output)
        self.assertIn("IF EIBRESP = 0", output)
        with self.assertRaisesRegex(MODULE.CicsError,
                                    "4: unsupported DFHRESP constant: NOTFOUND"):
            MODULE.transform(prefix +
                             "           IF EIBRESP = DFHRESP(NOTFOUND)\n",
                             self.maps)

    def test_receive_without_resp_requires_normal_result(self):
        prefix = ("       DATA DIVISION.\n"
                  "       WORKING-STORAGE SECTION.\n"
                  "       PROCEDURE DIVISION.\n")
        source = (prefix + "           EXEC CICS RECEIVE MAP('BNK1ME')\n"
                  "             END-EXEC.\n")
        output = MODULE.transform(source, self.maps)
        self.assertIn('CALL STATIC "COBCICS_REQUIRE_NORMAL"', output)
        self.assertLess(output.index('COBCICS_RECEIVE_STATUS'),
                        output.index('COBCICS_REQUIRE_NORMAL'))

    def test_send_control_rejects_unimplemented_options(self):
        prefix = ("       DATA DIVISION.\n"
                  "       WORKING-STORAGE SECTION.\n"
                  "       PROCEDURE DIVISION.\n")
        for command, expected in [
            ("SEND CONTROL ERASE", "requires ERASE FREEKB"),
            ("SEND CONTROL ERASE FREEKB ALARM", "unsupported SEND CONTROL option: ALARM"),
            ("SEND CONTROL ERASE(1) FREEKB", "do not take values"),
        ]:
            with self.subTest(command=command):
                with self.assertRaisesRegex(MODULE.CicsError, f"4: .*{expected}"):
                    MODULE.transform(prefix + "           EXEC CICS " + command +
                                     "\n             END-EXEC.\n", self.maps)

    def test_send_text_uses_source_length_and_rejects_unsupported_modes(self):
        output = "".join(MODULE.statement(
            "SEND TEXT FROM(END-MESSAGE) ERASE FREEKB RESP(STATUS)",
            self.maps, True))
        self.assertIn("MOVE LENGTH OF END-MESSAGE", output)
        self.assertIn('CALL STATIC "COBCICS_SEND_TEXT"', output)
        self.assertIn("MOVE EIBRESP TO STATUS", output)
        spaced = "".join(MODULE.statement(
            "SEND TEXT FROM (END-MESSAGE) ERASE FREEKB",
            self.maps, True))
        self.assertIn("USING END-MESSAGE COBCICS-LENGTH", spaced)
        for command, expected in [
            ("SEND TEXT FROM(MESSAGE) FREEKB", "requires ERASE FREEKB"),
            ("SEND TEXT FROM(MESSAGE) ERASE FREEKB PAGING", "unsupported SEND TEXT option: PAGING"),
            ("SEND TEXT FROM(MESSAGE) LENGTH(1920) ERASE FREEKB", "exceeds 1919"),
        ]:
            with self.subTest(command=command):
                with self.assertRaisesRegex(MODULE.CicsError, expected):
                    MODULE.statement(command, self.maps, True)

    def test_send_map_modes_and_invalid_combinations(self):
        prefix = ("       DATA DIVISION.\n"
                  "       WORKING-STORAGE SECTION.\n"
                  "       PROCEDURE DIVISION.\n")
        for options, action in (("ERASE FREEKB", "00"),
                                ("DATAONLY FREEKB", "02"),
                                ("DATAONLY ALARM FREEKB", "03")):
            with self.subTest(options=options):
                source = (prefix + "           EXEC CICS SEND MAP('BNK1ME')\n"
                          f"             {options} END-EXEC.\n")
                self.assertIn(f"USING X'{action}'",
                              MODULE.transform(source, self.maps))
        for options, expected in (("ERASE DATAONLY", "cannot be combined"),
                                  ("ALARM", "requires DATAONLY"),
                                  ("DATAONLY(1)", "does not take a value")):
            with self.subTest(options=options):
                source = (prefix + "           EXEC CICS SEND MAP('BNK1ME')\n"
                          f"             {options} END-EXEC.\n")
                with self.assertRaisesRegex(MODULE.CicsError, expected):
                    MODULE.transform(source, self.maps)

    def test_assign_rejects_unknown_option_and_long_program_id(self):
        prefix = ("       IDENTIFICATION DIVISION.\n"
                  "       PROGRAM-ID. LONGPROGRAM.\n"
                  "       DATA DIVISION.\n"
                  "       WORKING-STORAGE SECTION.\n"
                  "       PROCEDURE DIVISION.\n")
        for command, expected in [
            ("ASSIGN USERID(TARGET)", "unsupported ASSIGN option: USERID"),
            ("ASSIGN PROGRAM(TARGET)", "requires a 1-8 character PROGRAM-ID"),
        ]:
            with self.subTest(command=command):
                with self.assertRaisesRegex(MODULE.CicsError, expected):
                    MODULE.transform(prefix +
                                     "           EXEC CICS " + command +
                                     " END-EXEC.\n", self.maps)

    def test_linked_program_uses_link_start_and_borrowed_commarea(self):
        source = (ROOT / "extras" / "cobcics" /
                  "menu-linked-cics.cob").read_text()
        output = MODULE.transform(source, self.maps)
        self.assertIn('CALL STATIC "COBCICS_LINK_START"', output)
        self.assertNotIn('CALL STATIC "COBCICS_START"', output)
        self.assertNotIn('COBCICS_GET_COMMAREA"', output)
        self.assertIn('CALL STATIC "COBCICS_RETURN"', output)
        self.assertNotIn('COBCICS_FINISH', output)

    def test_link_requires_commarea_and_rejects_unsupported_options(self):
        prefix = ("       DATA DIVISION.\n"
                  "       WORKING-STORAGE SECTION.\n"
                  "       PROCEDURE DIVISION.\n")
        for command, expected in [
            ("LINK PROGRAM('OTHER')", "requires PROGRAM and COMMAREA"),
            ("LINK PROGRAM('OTHER') COMMAREA(AREA) CHANNEL('C')",
             "unsupported LINK option: CHANNEL"),
            ("LINK PROGRAM('OTHER') COMMAREA(AREA) LENGTH(32768)",
             "exceeds 32767"),
        ]:
            with self.subTest(command=command):
                fixed = command.replace(" COMMAREA", "\n             COMMAREA")
                fixed = fixed.replace(" LENGTH", "\n             LENGTH")
                fixed = fixed.replace(" CHANNEL", "\n             CHANNEL")
                with self.assertRaisesRegex(MODULE.CicsError, expected):
                    MODULE.transform(prefix + "           EXEC CICS " + fixed +
                                     "\n             END-EXEC.\n", self.maps)

    def test_return_requires_paired_bounded_commarea(self):
        prefix = ("       DATA DIVISION.\n"
                  "       WORKING-STORAGE SECTION.\n"
                  "       PROCEDURE DIVISION.\n")
        for command, expected in [
            ("RETURN TRANSID('OMEN') LENGTH(1)", "LENGTH requires COMMAREA"),
            ("RETURN TRANSID('OMEN') COMMAREA(AREA) LENGTH(2049)", "exceeds 2048"),
            ("RETURN COMMAREA(AREA) LENGTH(1)", "require TRANSID"),
            ("RETURN IMMEDIATE", "require TRANSID"),
            ("RETURN TRANSID('ODCS') IMMEDIATE(1)", "does not take a value"),
        ]:
            with self.subTest(command=command):
                with self.assertRaisesRegex(MODULE.CicsError, f"4: .*{expected}"):
                    fixed_command = command.replace(" COMMAREA", "\n             COMMAREA")
                    fixed_command = fixed_command.replace(" LENGTH", "\n             LENGTH")
                    MODULE.transform(prefix +
                                     "           EXEC CICS " + fixed_command +
                                     "\n             END-EXEC.\n", self.maps)

    def test_return_immediate_emits_dispatch_marker(self):
        output = MODULE.statement("RETURN TRANSID('ODCS') IMMEDIATE",
                                  self.maps, True)
        joined = "".join(output)
        self.assertLess(joined.index('COBCICS_SET_RETURN'),
                        joined.index('COBCICS_SET_IMMEDIATE'))
        self.assertLess(joined.index('COBCICS_SET_IMMEDIATE'),
                        joined.index('COBCICS_RETURN'))

    def test_return_accepts_variable_transid(self):
        output = MODULE.statement("RETURN TRANSID(WS-CICS-TRANID) COMMAREA(AREA)",
                                  self.maps, False)
        self.assertIn("USING WS-CICS-TRANID AREA COBCICS-LENGTH", "".join(output))

    def test_bank_menu_all_cics_blocks_translate(self):
        source = (ROOT / "reference" / "Bank-of-Z-main" / "src" /
                  "base" / "cics" / "cobol" / "BNKMENU.cbl").read_text()
        output = MODULE.transform(source, self.maps, "BNK1MAI")
        self.assertNotIn("EXEC CICS", output)
        for call in ("COBCICS_SEND_TEXT", "COBCICS_ABEND",
                     "COBCICS_ASKTIME", "COBCICS_FORMATTIME"):
            self.assertIn(f'CALL STATIC "{call}"', output)

    def test_display_account_new_commands_translate(self):
        deedit = "".join(MODULE.statement("BIF DEEDIT FIELD(ACCNOI)", self.maps, True))
        self.assertIn('CALL STATIC "COBCICS_DEEDIT"', deedit)
        self.assertIn("MOVE LENGTH OF ACCNOI", deedit)
        link = "".join(MODULE.statement(
            "LINK PROGRAM('INQACC') COMMAREA(AREA) SYNCONRETURN",
            self.maps, True))
        self.assertIn('CALL "INQACC" USING AREA', link)
        self.assertNotIn("UNSUPPORTED_SYNC", link)
        with self.assertRaisesRegex(MODULE.CicsError, "unsupported LINK option: SYSID"):
            MODULE.statement(
                "LINK PROGRAM('INQACC') COMMAREA(AREA) "
                "SYSID('CICR') SYNCONRETURN", self.maps, True)

    def test_time_and_abend_reject_unsupported_shapes(self):
        for command, expected in [
            ("ABEND ABCODE('TOOLONG')", "requires a 1-4 character literal"),
            ("ABEND ABCODE('HBNK') CANCEL(1)", "CANCEL does not take a value"),
            ("SYNCPOINT ROLLBACK(1)", "ROLLBACK takes no value"),
            ("DELAY FOR SECONDS(61)", "requires 0-60"),
            ("ASKTIME ABSTIME(CLOCK) TIME(DATE)", "unsupported ASKTIME option"),
            ("FORMATTIME ABSTIME(CLOCK) DDMMYYYY(DATE)", "TIME requires"),
            ("FORMATTIME ABSTIME(CLOCK) DDMMYYYY(DATE) TIME(NOW) DATESEP('AB')",
             "DATESEP requires a single character literal"),
        ]:
            with self.subTest(command=command):
                with self.assertRaisesRegex(MODULE.CicsError, expected):
                    MODULE.statement(command, self.maps, True)

    def test_bank_abend_handler_and_syncpoint(self):
        source = (ROOT / "reference" / "Bank-of-Z-main" / "src" /
                  "base" / "cics" / "cobol" / "XFRFUN.cbl").read_text()
        output = MODULE.transform(source, self.maps)
        self.assertIn("GO TO ABEND-HANDLING", output)
        self.assertIn("MOVE COBCICS-ABEND-CODE TO MY-ABEND-CODE", output)
        self.assertIn('CALL STATIC "COBCICS_SYNCPOINT"', output)
        self.assertIn("USING X'01' EIBRESP EIBRESP2", output)
        self.assertIn('CALL STATIC "COBCICS_DELAY"', output)
        self.assertNotIn("EXEC CICS", output)
        abend_source = (ROOT / "reference" / "Bank-of-Z-main" / "src" /
                        "base" / "cics" / "cobol" / "ABNDPROC.cbl").read_text()
        self.assertIn('CALL STATIC "COBCICS_WRITE_ABNDFILE"',
                      MODULE.transform(abend_source, self.maps))
        cancel = "".join(MODULE.statement(
            "ABEND ABCODE('HBNK') CANCEL", self.maps, True,
            abend_label="ABEND-HANDLING"))
        self.assertNotIn("GO TO ABEND-HANDLING", cancel)

    def test_implicit_linked_commarea_binds_using(self):
        source = (ROOT / "reference" / "Bank-of-Z-main" / "src" /
                  "base" / "cics" / "cobol" / "UPDACC.cbl").read_text()
        output = MODULE.transform(source, self.maps)
        self.assertIn("PROCEDURE DIVISION USING DFHCOMMAREA.", output)
        self.assertIn('CALL STATIC "COBCICS_LINK_START"', output)
        self.assertNotIn('CALL STATIC "COBCICS_START"', output)


if __name__ == "__main__":
    unittest.main()
