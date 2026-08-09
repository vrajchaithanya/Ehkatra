"""Builds the XLSX starter corpus (BOOTSTRAP row 12: "round-trip corpus starter
(20 files)").

Written by hand rather than by a spreadsheet library, for the same reason the
ZIP corpus is built by Python's `zipfile`: a reader tested against files its own
writer produced proves that two bugs agree. These parts are assembled from the
ECMA-376 shapes Excel actually emits, including the ones that are awkward on
purpose — inline strings, cached formula results, error cells, custom number
formats, a sheet out of relationship order, and active content that must be
quarantined rather than read.

Run: python crates/usk-xlsx/tests/make_corpus.py
The output is committed; this script exists so the corpus can be regenerated and
argued with, not so it is rebuilt on every test run.
"""

import os
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "corpus")

CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
</Types>"""

ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"""


def workbook(sheets):
    entries = "".join(
        '<sheet name="%s" sheetId="%d" r:id="rId%d"/>' % (name, i + 1, i + 1)
        for i, name in enumerate(sheets)
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"'
        ' xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        "<sheets>%s</sheets></workbook>" % entries
    )


def workbook_rels(count, targets=None):
    targets = targets or ["worksheets/sheet%d.xml" % (i + 1) for i in range(count)]
    entries = "".join(
        '<Relationship Id="rId%d" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="%s"/>'
        % (i + 1, target)
        for i, target in enumerate(targets)
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        "%s</Relationships>" % entries
    )


def sheet(rows):
    """rows: list of (row_number, [cell_xml, ...])"""
    body = "".join(
        '<row r="%d">%s</row>' % (n, "".join(cells)) for n, cells in rows
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        "<sheetData>%s</sheetData></worksheet>" % body
    )


def shared_strings(items):
    body = "".join("<si><t>%s</t></si>" % item for item in items)
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="%d" uniqueCount="%d">%s</sst>'
        % (len(items), len(items), body)
    )


def styles(cell_xfs, num_fmts=()):
    fmts = "".join(
        '<numFmt numFmtId="%d" formatCode="%s"/>' % (i, code) for i, code in num_fmts
    )
    xfs = "".join('<xf numFmtId="%d" xfId="0"/>' % fmt for fmt in cell_xfs)
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        '<numFmts count="%d">%s</numFmts>'
        '<cellXfs count="%d">%s</cellXfs></styleSheet>'
        % (len(num_fmts), fmts, len(cell_xfs), xfs)
    )


def write(name, parts, compression=zipfile.ZIP_DEFLATED):
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, name)
    with zipfile.ZipFile(path, "w", compression) as z:
        for part, body in parts.items():
            data = body if isinstance(body, bytes) else body.encode("utf-8")
            z.writestr(zipfile.ZipInfo(part), data, compression)
    return path


def base(sheet_bodies, extra=None, sheet_names=None, rels_targets=None):
    names = sheet_names or ["Sheet%d" % (i + 1) for i in range(len(sheet_bodies))]
    parts = {
        "[Content_Types].xml": CONTENT_TYPES,
        "_rels/.rels": ROOT_RELS,
        "xl/workbook.xml": workbook(names),
        "xl/_rels/workbook.xml.rels": workbook_rels(len(sheet_bodies), rels_targets),
    }
    targets = rels_targets or ["worksheets/sheet%d.xml" % (i + 1) for i in range(len(sheet_bodies))]
    for target, body in zip(targets, sheet_bodies):
        parts["xl/" + target] = body
    parts.update(extra or {})
    return parts


def main():
    # 01 — the simplest possible workbook: one numeric cell.
    write("01-minimal.xlsx", base([sheet([(1, ['<c r="A1"><v>42</v></c>'])])]))

    # 02 — numbers of several shapes, including the ones that stress f64.
    write("02-numbers.xlsx", base([sheet([
        (1, ['<c r="A1"><v>0</v></c>', '<c r="B1"><v>-1.5</v></c>',
             '<c r="C1"><v>1e-17</v></c>', '<c r="D1"><v>1234567890123456</v></c>']),
        (2, ['<c r="A2"><v>0.1</v></c>', '<c r="B2"><v>3.141592653589793</v></c>']),
    ])]))

    # 03 — shared strings, the table that turns a workbook of numbers into text.
    write("03-shared-strings.xlsx", base(
        [sheet([(1, ['<c r="A1" t="s"><v>0</v></c>', '<c r="B1" t="s"><v>1</v></c>']),
                (2, ['<c r="A2" t="s"><v>0</v></c>'])])],
        {"xl/sharedStrings.xml": shared_strings(["hello", "world"])}))

    # 04 — formulas with their cached results, which is what XLSX stores.
    write("04-formulas.xlsx", base([sheet([
        (1, ['<c r="A1"><v>2</v></c>', '<c r="B1"><v>3</v></c>',
             '<c r="C1"><f>A1+B1</f><v>5</v></c>']),
        (2, ['<c r="A2"><f>SUM(A1:B1)</f><v>5</v></c>']),
    ])]))

    # 05 — error cells, every spelling Excel writes.
    write("05-errors.xlsx", base([sheet([(1, [
        '<c r="A1" t="e"><f>1/0</f><v>#DIV/0!</v></c>',
        '<c r="B1" t="e"><v>#N/A</v></c>',
        '<c r="C1" t="e"><v>#REF!</v></c>',
        '<c r="D1" t="e"><v>#NAME?</v></c>',
        '<c r="E1" t="e"><v>#VALUE!</v></c>',
    ])])]))

    # 06 — booleans.
    write("06-booleans.xlsx", base([sheet([(1, [
        '<c r="A1" t="b"><v>1</v></c>', '<c r="B1" t="b"><v>0</v></c>',
        '<c r="C1" t="b"><f>1=1</f><v>1</v></c>',
    ])])]))

    # 07 — inline strings: text stored in the cell rather than the shared table.
    write("07-inline-strings.xlsx", base([sheet([(1, [
        '<c r="A1" t="inlineStr"><is><t>inline</t></is></c>',
        '<c r="B1" t="str"><f>UPPER("x")</f><v>X</v></c>',
    ])])]))

    # 08 — number formats: built-in ids and a custom code.
    write("08-number-formats.xlsx", base(
        [sheet([(1, ['<c r="A1" s="1"><v>1234.5</v></c>',
                     '<c r="B1" s="2"><v>45000</v></c>',
                     '<c r="C1" s="3"><v>0.25</v></c>',
                     '<c r="D1" s="0"><v>7</v></c>'])])],
        {"xl/styles.xml": styles([0, 2, 14, 164], [(164, "&quot;$&quot;#,##0.00")])}))

    # 09 — several sheets.
    write("09-multi-sheet.xlsx", base([
        sheet([(1, ['<c r="A1"><v>1</v></c>'])]),
        sheet([(1, ['<c r="A1"><v>2</v></c>'])]),
        sheet([(1, ['<c r="A1"><v>3</v></c>'])]),
    ], sheet_names=["First", "Second", "Third"]))

    # 10 — relationship targets out of order, so a reader that assumes
    # sheetN.xml matches sheet N gets the wrong sheet.
    write("10-rels-out-of-order.xlsx", base(
        # Bodies are paired with `rels_targets`, so sheetB.xml holds 222 and
        # sheetA.xml holds 111 — the crossover a filename-assuming reader gets
        # backwards.
        [sheet([(1, ['<c r="A1"><v>222</v></c>'])]),
         sheet([(1, ['<c r="A1"><v>111</v></c>'])])],
        sheet_names=["Alpha", "Beta"],
        rels_targets=["worksheets/sheetB.xml", "worksheets/sheetA.xml"]))

    # 11 — a sparse sheet: gaps in rows and columns, high column letters.
    write("11-sparse.xlsx", base([sheet([
        (1, ['<c r="A1"><v>1</v></c>']),
        (5, ['<c r="C5"><v>5</v></c>']),
        (100, ['<c r="AA100"><v>100</v></c>', '<c r="ZZ100"><v>702</v></c>']),
    ])]))

    # 12 — XML entities and unicode in shared strings.
    write("12-entities.xlsx", base(
        [sheet([(1, ['<c r="A1" t="s"><v>0</v></c>', '<c r="B1" t="s"><v>1</v></c>',
                     '<c r="C1" t="s"><v>2</v></c>'])])],
        {"xl/sharedStrings.xml": shared_strings(
            ["a &amp; b &lt; c", "caf&#233;", "&#128512;"])}))

    # 13 — active content. Must be quarantined, never decompressed.
    write("13-macro-enabled.xlsm", base(
        [sheet([(1, ['<c r="A1"><v>1</v></c>'])])],
        {"xl/vbaProject.bin": b"\xd0\xcf\x11\xe0" + b"MACRO PAYLOAD" * 20}))

    # 14 — parts v0.1 does not model: chart, drawing, theme.
    write("14-unmodelled-parts.xlsx", base(
        [sheet([(1, ['<c r="A1"><v>1</v></c>'])])],
        {"xl/charts/chart1.xml": "<chart/>",
         "xl/drawings/drawing1.xml": "<drawing/>",
         "xl/theme/theme1.xml": "<theme/>"}))

    # 15 — stored (uncompressed) parts: a legal ZIP that some writers produce.
    write(
        "15-stored.xlsx",
        base([sheet([(1, ['<c r="A1"><v>15</v></c>'])])]),
        compression=zipfile.ZIP_STORED,
    )

    # 16 — a cell with a style index that styles.xml does not define.
    write("16-dangling-style.xlsx", base(
        [sheet([(1, ['<c r="A1" s="99"><v>1</v></c>'])])],
        {"xl/styles.xml": styles([0])}))

    # 17 — a shared-string index past the end of the table.
    write("17-bad-shared-index.xlsx", base(
        [sheet([(1, ['<c r="A1" t="s"><v>0</v></c>', '<c r="B1" t="s"><v>99</v></c>'])])],
        {"xl/sharedStrings.xml": shared_strings(["only one"])}))

    # 18 — a cell type this build does not model, and a malformed reference.
    write("18-odd-cells.xlsx", base([sheet([(1, [
        '<c r="A1" t="mystery"><v>?</v></c>',
        '<c r="NOTAREF"><v>1</v></c>',
        '<c r="B1"><v>ok</v></c>',
    ])])]))

    # 19 — no sharedStrings and no styles parts at all: both are optional.
    write("19-no-optional-parts.xlsx", {
        "[Content_Types].xml": CONTENT_TYPES,
        "_rels/.rels": ROOT_RELS,
        "xl/workbook.xml": workbook(["Only"]),
        "xl/_rels/workbook.xml.rels": workbook_rels(1),
        "xl/worksheets/sheet1.xml": sheet([(1, ['<c r="A1"><v>19</v></c>'])]),
    })

    # 20 — a workbook whose relationship part is missing, so the reader must
    # fall back to Excel's naming convention rather than lose the sheet.
    write("20-missing-rels.xlsx", {
        "[Content_Types].xml": CONTENT_TYPES,
        "_rels/.rels": ROOT_RELS,
        "xl/workbook.xml": workbook(["Recovered"]),
        "xl/worksheets/sheet1.xml": sheet([(1, ['<c r="A1"><v>20</v></c>'])]),
    })

    print("wrote", len(os.listdir(OUT)), "files to", OUT)


if __name__ == "__main__":
    main()
