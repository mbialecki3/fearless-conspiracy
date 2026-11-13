#!/usr/bin/env python3
"""Build the improved-game report and poster using ReportLab.

Run with a Python environment containing reportlab, Pillow, and pypdf.
The report text lives in docs/REPORT.md; screenshots live in docs/screenshots.
The generated PDFs are written to submission/.
"""
from html import escape
from pathlib import Path
import json
import re

from PIL import Image as PILImage
from reportlab.lib import colors
from reportlab.lib.styles import ParagraphStyle
from reportlab.platypus import SimpleDocTemplate, Paragraph, Spacer, Image, PageBreak, KeepTogether
from reportlab.pdfgen import canvas
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'submission'

# Embed fonts so viewers do not substitute incompatible metrics.
FONTDIR = Path('/usr/share/fonts/truetype/liberation')
if not FONTDIR.exists():
    FONTDIR = Path('/usr/share/fonts/truetype/liberation2')
for name, filename in [('Times-Roman','LiberationSerif-Regular.ttf'),
                       ('Times-Bold','LiberationSerif-Bold.ttf'),
                       ('Helvetica','LiberationSans-Regular.ttf'),
                       ('Helvetica-Bold','LiberationSans-Bold.ttf')]:
    pdfmetrics.registerFont(TTFont(name, str(FONTDIR / filename)))
pdfmetrics.registerFont(TTFont('Courier','/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf'))


def inline(value):
    parts = re.split(r'(`[^`]+`)', ' '.join(value.splitlines()))
    return ''.join('<font name="Courier" size="9">' + escape(p[1:-1]) + '</font>'
                   if p.startswith('`') else escape(p) for p in parts)


def report():
    styles = {
        'body': ParagraphStyle('body', fontName='Times-Roman', fontSize=11, leading=13.7, spaceAfter=8),
        'title': ParagraphStyle('title', fontName='Times-Bold', fontSize=15, leading=19, spaceAfter=10, keepWithNext=True),
        'head': ParagraphStyle('head', fontName='Times-Bold', fontSize=12, leading=15, spaceBefore=8, spaceAfter=7, keepWithNext=True),
        'caption': ParagraphStyle('caption', fontName='Times-Roman', fontSize=10, leading=12.5, spaceBefore=5, spaceAfter=12),
    }
    path = OUT / 'Fearless-Conspiracy-Report-Draft.pdf'
    doc = SimpleDocTemplate(str(path), pagesize=(612,792), leftMargin=54, rightMargin=54,
                            topMargin=43, bottomMargin=45, author='Mitchell Bialecki',
                            title='Fearless Conspiracy: Linux Isolation - Report Draft')
    story = []
    blocks = re.split(r'\n\s*\n', (ROOT/'docs/REPORT.md').read_text().strip())
    i = 0
    while i < len(blocks):
        b = blocks[i].strip()
        if b == '<!-- pagebreak -->':
            story.append(PageBreak())
        elif b.startswith('# '):
            story.append(Paragraph(inline(b[2:]), styles['title']))
        elif b.startswith('## '):
            story.append(Paragraph(inline(b[3:]), styles['head']))
        elif b.startswith('!['):
            m = re.fullmatch(r'!\[.*?\]\((.*?)\)', b)
            p = (ROOT/'docs'/m.group(1)).resolve()
            with PILImage.open(p) as image:
                w,h = image.size
            scale = min(504/w, 265/h)
            pic = Image(str(p), width=w*scale, height=h*scale)
            pic.hAlign='LEFT'
            i += 1
            assert blocks[i].startswith('Figure '), p
            story.append(KeepTogether([pic, Paragraph(inline(blocks[i]),styles['caption'])]))
        else:
            story.append(Paragraph(inline(b),styles['body']))
        i += 1
    def footer(c,d):
        c.setFont('Times-Roman',9)
        c.drawString(54,25,'Mitchell Bialecki | CSE 536 | Report draft')
        c.drawRightString(558,25,str(d.page))
    doc.build(story,onFirstPage=footer,onLaterPages=footer)
    print(path)


def poster():
    proc = json.loads((ROOT/'docs/evidence/proc/evidence.json').read_text())
    verified = json.loads((ROOT/'docs/evidence/verification.json').read_text())
    assert proc['pass'] and verified['restores_prior_settings']
    windows = proc['measurements']
    before = [m for m in windows if m['label'].startswith('hidepid0_')]
    protected = [m for m in windows if m['label'].startswith('hidepid2_')]
    recovery = [m for m in windows if m['label'].startswith('restored0_')]
    assert (len(before), len(protected), len(recovery)) == (8,8,6)
    assert all(m['pass'] for m in windows)
    probe = next(m for m in before if m['label'] == 'hidepid0_mafia_1')
    active = set(probe['inferred_active_recipients'])
    assert active == set(probe['expected_recipients_validation_only'])
    path = OUT / 'Fearless-Conspiracy-Poster.pdf'
    c = canvas.Canvas(str(path),pagesize=(1728,1296))
    c.setTitle('Private messages can leak without being read')
    c.setAuthor('Mitchell Bialecki')
    ink = colors.HexColor('#172832')
    accent = colors.HexColor('#23576b')
    pale = colors.HexColor('#edf2f4')
    muted = colors.HexColor('#c9d0d4')
    c.setFillColor(ink)
    c.setFont('Helvetica-Bold',59)
    c.drawString(68,1185,'Private messages can leak')
    c.drawString(68,1116,'without being read')
    c.setFont('Helvetica',26)
    c.drawString(70,1066,'Mitchell Bialecki  |  CSE 536  |  Fearless Conspiracy')
    c.setStrokeColor(accent)
    c.setLineWidth(3)
    c.line(70,1030,1658,1030)

    def para(text,x,y,width,size=25,leading=34,bold=False):
        style = ParagraphStyle('poster',fontName='Helvetica-Bold' if bold else 'Helvetica',
                               fontSize=size,leading=leading,textColor=ink)
        p = Paragraph(escape(text),style)
        _,h = p.wrap(width,1000)
        assert y-h > 30, (text, y, h)
        p.drawOn(c,x,y-h)
        return y-h

    cols = [70,612,1154]
    width = 490
    for x,title in zip(cols,['The problem','The change','The result']):
        c.setFillColor(accent)
        c.setFont('Helvetica-Bold',34)
        c.drawString(x,968,title)

    para('Mafia messages reached the right players. But a Town player could watch server threads and infer which connections received them.',cols[0],925,width)
    c.setFillColor(ink)
    c.setFont('Helvetica-Bold',24)
    c.drawString(cols[0],752,'One controlled Mafia-chat window')
    for player in range(1,10):
        x = cols[0] + 75 + ((player-1)%3)*150
        y = 675 - ((player-1)//3)*95
        c.setFillColor(accent if player in active else pale)
        c.setStrokeColor(accent if player in active else muted)
        c.setLineWidth(1.5)
        c.circle(x,y,30,fill=1,stroke=1)
        c.setFillColor(colors.white if player in active else ink)
        c.setFont('Helvetica-Bold',25)
        c.drawCentredString(x,y-9,str(player))
    para('Highlighted IDs = active connections',cols[0],425,width,22,29)
    para('Public joins mapped threads to players. The observer never received the Mafia text.',cols[0],381,width,22,29)

    para('Protected startup is the default. Separate Linux accounts and hidepid=2 block access to the server metadata used by this test.',cols[1],925,width)
    c.setFillColor(pale)
    c.rect(cols[1],716,width,58,fill=1,stroke=0)
    c.setFillColor(ink)
    c.setFont('Courier',23)
    c.drawString(cols[1]+15,737,'sudo ./server --players 5')
    c.setFont('Helvetica',23)
    c.drawString(cols[1],679,'Resource limits per player account')
    for y,label in [(624,'25% of one CPU'),(567,'256 MiB hard memory cap'),(510,'64 tasks')]:
        c.setFillColor(accent)
        c.setFont('Helvetica-Bold',30)
        c.drawString(cols[1],y,label)
    para("Other players' cgroup statistics are restricted too.",cols[1],465,width,24,32)

    for y,value,description in [
        (865,'8 / 8','Baseline windows matched expected recipients, including idle controls.'),
        (681,'8 / 8','Protected windows: the required metadata was unavailable.'),
        (497,'6 / 6','After visibility was re-enabled, the inference matched again.')]:
        c.setFillColor(accent)
        c.setFont('Helvetica-Bold',59)
        c.drawString(cols[2],y,value)
        para(description,cols[2],y-26,width,23,30)

    c.setFillColor(pale)
    c.rect(70,211,1588,108,fill=1,stroke=0)
    para('Protect while playing. Restore when finished.',95,302,1530,33,40,bold=True)
    para('Shutdown restores prior settings. Forced-kill recovery uses --restore.',95,253,1530,24,30)
    para('Resource tests throttled CPU, blocked forks, and killed the memory stressor. The game finished.',70,188,1588,23,29)
    para('Hard-memory test: the 192 MiB soft throttle was temporarily disabled inside the test VM.',70,146,1588,18,23)
    para('Controlled quiet-window tests, November 12, 2026. This blocks the measured channel; it does not prove every side channel is gone.',70,112,1588,18,23)
    para('I learned to protect activity as well as message contents.',70,77,1588,25,31,bold=True)
    c.showPage()
    c.save()
    print(path)


if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    report();poster()
