#!/usr/bin/env python3
from pathlib import Path

from reportlab.lib import colors
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import getSampleStyleSheet
from reportlab.lib.units import cm
from reportlab.platypus import Paragraph, SimpleDocTemplate, Spacer


HERE = Path(__file__).resolve().parent
SRC = HERE / "SYNAPSE_ENTERPRISE_SECURITY.md"
OUT = HERE / "SYNAPSE_ENTERPRISE_SECURITY.pdf"


def blocks(markdown: str):
    styles = getSampleStyleSheet()
    styles["Title"].fontName = "Helvetica-Bold"
    styles["Heading1"].textColor = colors.HexColor("#111827")
    body = styles["BodyText"]
    body.leading = 14
    out = []
    for raw in markdown.splitlines():
        line = raw.strip()
        if not line:
            out.append(Spacer(1, 0.18 * cm))
        elif line.startswith("# "):
            out.append(Paragraph(line[2:], styles["Title"]))
            out.append(Spacer(1, 0.25 * cm))
        elif line.startswith("## "):
            out.append(Spacer(1, 0.12 * cm))
            out.append(Paragraph(line[3:], styles["Heading1"]))
        elif line.startswith("- "):
            out.append(Paragraph("• " + line[2:], body))
        elif line[0:2].isdigit() and ". " in line[:4]:
            out.append(Paragraph(line, body))
        else:
            out.append(Paragraph(line, body))
    return out


def main():
    doc = SimpleDocTemplate(
        str(OUT),
        pagesize=A4,
        leftMargin=1.8 * cm,
        rightMargin=1.8 * cm,
        topMargin=1.7 * cm,
        bottomMargin=1.7 * cm,
        title="Synapse Enterprise Security Brief",
        author="Synapse",
    )
    doc.build(blocks(SRC.read_text(encoding="utf-8")))
    print(OUT)


if __name__ == "__main__":
    main()
