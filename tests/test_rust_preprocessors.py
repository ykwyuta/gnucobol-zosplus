"""Compare the Rust command line translators with the previous implementations.

The Python modules are retained only as regression oracles; cobc invokes Rust.
"""

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "extras" / "preprocessors" / "target" / "debug"
SUFFIX = ".exe" if os.name == "nt" else ""


def legacy(name):
    path = ROOT / "tests" / "legacy_preprocessors" / f"{name}.py"
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def translated(name, source, *options):
    with tempfile.TemporaryDirectory() as directory:
        source_path = Path(directory) / "source.cbl"
        output_path = Path(directory) / "output.cbl"
        source_path.write_text(source, encoding="utf-8")
        arguments = []
        for option in options:
            if isinstance(option, tuple):
                filename, data = option
                path = Path(directory) / filename
                path.write_text(data, encoding="utf-8")
                arguments.append(str(path))
            else:
                arguments.append(option)
        if name == "cics-preprocess":
            arguments = [str(source_path), arguments[0], str(output_path)]
        elif name == "imsdli-preprocess":
            arguments = [str(source_path), str(output_path), "--dbd", arguments[0]]
        else:
            arguments = [*arguments, str(source_path), str(output_path)]
        result = subprocess.run([str(BIN / (name + SUFFIX)), *arguments],
                                capture_output=True, text=True)
        if result.returncode:
            raise AssertionError(result.stderr)
        return output_path.read_text(encoding="utf-8")


class RustPreprocessors(unittest.TestCase):
    def test_cics(self):
        module = legacy("cics-preprocess")
        copybook = ("       01 BNK1MEI.\n"
                    "       02 ACTIONL PIC S9(4) COMP.\n"
                    "       02 ACTIONI PIC X(1).\n"
                    "       01 BNK1MEO REDEFINES BNK1MEI.\n"
                    "       02 ACTIONO PIC X(1).\n")
        source = (ROOT / "extras/cobcics/menu-worker-cics.cob").read_text()
        expected = module.transform(source, module.maps_from_copybook(copybook))
        actual = translated("cics-preprocess", source, ("BNK1MAI.cpy", copybook))
        self.assertEqual(actual, expected)

    def test_sql(self):
        module = legacy("db2sql-preprocess")
        for fixture in ("demo.cbl", "phase2-demo.cbl", "ebcdic-demo.cbl",
                        "varchar-demo.cbl"):
            with self.subTest(fixture=fixture):
                source = (ROOT / "extras/db2sql" / fixture).read_text(encoding="utf-8")
                self.assertEqual(translated("db2sql-preprocess", source,
                                            "-I", str(ROOT / "extras/db2sql")),
                                 module.transform(source,
                                                  [ROOT / "extras/db2sql"]))

    def test_ims_dli(self):
        module = legacy("imsdli-preprocess")
        source = ("       WORKING-STORAGE SECTION.\n"
                  "       LINKAGE SECTION.\n"
                  "           EXEC DLI GU USING PCB(PAUT-PCB-NUM)\n"
                  "               SEGMENT(PAUTSUM0) INTO(DATA)\n"
                  "               WHERE(ACCNTID = PA-ACCT-ID) END-EXEC\n"
                  "           EXEC DLI CHKP ID(WK-CHKPT-ID) END-EXEC\n")
        self.assertEqual(translated("imsdli-preprocess", source, "DBPAUTP0"),
                         module.translate(source, "DBPAUTP0"))


if __name__ == "__main__":
    unittest.main()
