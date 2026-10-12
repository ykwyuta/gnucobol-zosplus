import importlib.util
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "cics_preprocess", ROOT / "tests" / "legacy_preprocessors" / "cics-preprocess.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class GenAppPreprocessTests(unittest.TestCase):
    def test_handle_aid_and_condition_branch(self):
        aid = "".join(MODULE.statement(
            "HANDLE AID CLEAR(CLEARIT) PF3(ENDIT)", {}, True
        ))
        self.assertIn("IF EIBAID = X'6D'", aid)
        self.assertIn("GO TO CLEARIT", aid)
        self.assertIn("IF EIBAID = X'F3'", aid)
        self.assertIn("GO TO ENDIT", aid)
        condition = "".join(MODULE.statement(
            "HANDLE CONDITION MAPFAIL(ENDIT)", {}, True
        ))
        self.assertEqual(condition, "           CONTINUE.\n")
        receive = "".join(MODULE.statement(
            "RECEIVE MAP('SSMAPC1') INTO(SSMAPC1I) MAPSET('SSMAP')",
            {"SSMAPC1": [("ENT1OPT", 1)]}, True,
            expected_mapset="SSMAP", mapfail_label="ENDIT",
        ))
        self.assertIn("IF EIBRESP = 36\n             GO TO ENDIT", receive)

    def test_genapp_length_of_and_month_first_date(self):
        date = "".join(MODULE.statement(
            "FORMATTIME ABSTIME(WS-ABSTIME) MMDDYYYY(WS-DATE) "
            "TIME(WS-TIME)", {}, False
        ))
        self.assertIn('CALL STATIC "COBCICS_FORMATTIME_MMDD"', date)
        link = "".join(MODULE.statement(
            "LINK PROGRAM('LGSTSQ') COMMAREA(ERROR-MSG) "
            "LENGTH(LENGTH OF ERROR-MSG)", {}, True
        ))
        self.assertIn("MOVE LENGTH OF ERROR-MSG TO COBCICS-LENGTH", link)
        message = "".join(MODULE.statement(
            "SEND TEXT FROM(MSGEND) LENGTH(LENGTH OF MSGEND) "
            "ERASE FREEKB", {}, True
        ))
        self.assertIn("MOVE LENGTH OF MSGEND TO COBCICS-LENGTH", message)

    def test_temporary_storage_queue_calls_are_not_empty_stubs(self):
        read = "".join(MODULE.statement(
            "READQ TS QUEUE(STSQ-NAME) INTO(READ-MSG) "
            "RESP(WS-RESP) ITEM(1)", {}, False
        ))
        self.assertIn('CALL STATIC "COBCICS_TSQ_READ"', read)
        self.assertIn("MOVE 1 TO COBCICS-QUEUE-ITEM", read)
        write = "".join(MODULE.statement(
            "WRITEQ TS QUEUE(STSQ-NAME) FROM(WRITE-MSG) "
            "LENGTH(24) REWRITE ITEM(WS-ITEM-COUNT) RESP(WS-RESP)",
            {}, False
        ))
        self.assertIn('CALL STATIC "COBCICS_TSQ_WRITE"', write)
        self.assertIn("MOVE 24", write)
        self.assertIn("X'01' EIBRESP EIBRESP2", write)
        delete = "".join(MODULE.statement(
            "DELETEQ TS QUEUE(STSQ-NAME) RESP(WS-RESP)", {}, False
        ))
        self.assertIn('CALL STATIC "COBCICS_TSQ_DELETE"', delete)
        variable = "".join(MODULE.statement(
            "WRITEQ TD QUEUE(TDQ-NAME) FROM(WRITE-MSG) "
            "LENGTH(WS-RECV-LEN) RESP(WS-RESP)", {}, False
        ))
        self.assertIn("MOVE WS-RECV-LEN\n"
                      "             TO COBCICS-QUEUE-LENGTH", variable)
        self.assertIn("WRITE-MSG\n"
                      "                   COBCICS-QUEUE-LENGTH", variable)
        read_variable = "".join(MODULE.statement(
            "READQ TS QUEUE(STSQ-NAME) INTO(READ-MSG) "
            "LENGTH(WS-RECV-LEN) RESP(WS-RESP)", {}, False
        ))
        self.assertIn("MOVE COBCICS-QUEUE-LENGTH\n"
                      "             TO WS-RECV-LEN", read_variable)

    def test_named_counter_and_implicit_channel(self):
        define = "".join(MODULE.statement(
            "DEFINE COUNTER(GENACNT100) POOL(GENAPOOL) "
            "VALUE(0) RESP(WS-RESP)", {}, False
        ))
        self.assertIn('CALL STATIC "COBCICS_COUNTER_DEFINE"', define)
        self.assertIn("MOVE 0\n             TO COBCICS-COUNTER-VALUE", define)
        for verb in ("GET", "QUERY", "DELETE"):
            source = (f"{verb} COUNTER(GENACNT100) POOL(GENAPOOL) "
                      + ("VALUE(COUNTVAL) " if verb != "DELETE" else "")
                      + "RESP(WS-RESP)")
            output = "".join(MODULE.statement(source, {}, False))
            self.assertIn(f'CALL STATIC "COBCICS_COUNTER_{verb}"', output)
        get = "".join(MODULE.statement(
            "GET CONTAINER(WS-NAME) INTO(WS-DATA) RESP(WS-RESP)",
            {}, False
        ))
        self.assertIn("TO COBCICS-DATA-LENGTH", get)
        self.assertIn("COBCICS-CURRENT-CHANNEL", get)

    def test_genapp_ksds_file_calls(self):
        read = "".join(MODULE.statement(
            "READ FILE('KSDSPOLY') INTO(CA-AREA) LENGTH(F64) "
            "RIDFLD(PART-KEY) KEYLENGTH(F11) GENERIC GTEQ "
            "RESP(WS-RESP)", {}, False
        ))
        self.assertIn('CALL STATIC "COBCICS_FILE_READ"', read)
        self.assertIn("X'03' EIBRESP EIBRESP2", read)
        self.assertIn("TO F64", read)
        update = "".join(MODULE.statement(
            "READ FILE('KSDSCUST') INTO(WS-CUSTOMER-AREA) "
            "LENGTH(WS-COMMAREA-LEN) RIDFLD(CA-CUSTOMER-NUM) "
            "KEYLENGTH(10) UPDATE RESP(WS-RESP)", {}, False
        ))
        self.assertIn("X'04' EIBRESP EIBRESP2", update)
        for verb, source in (
            ("WRITE", "WRITE FILE('KSDSCUST') FROM(CA-CUSTOMER-NUM) "
             "LENGTH(225) RIDFLD(CA-CUSTOMER-NUM) KEYLENGTH(10)"),
            ("REWRITE", "REWRITE FILE('KSDSCUST') "
             "FROM(CA-CUSTOMER-NUM) LENGTH(225)"),
            ("DELETE", "DELETE FILE('KSDSPOLY') "
             "RIDFLD(WF-POLICY-KEY) KEYLENGTH(21)"),
        ):
            self.assertIn(f'CALL STATIC "COBCICS_FILE_{verb}"',
                          "".join(MODULE.statement(source, {}, False)))


if __name__ == "__main__":
    unittest.main()
