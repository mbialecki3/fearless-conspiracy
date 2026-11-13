#!/usr/bin/env python3
"""Create a transcript viewer of actual saved game output for browser screenshots.

This selects real lines and labels the excerpts. It does not synthesize gameplay.
Run this script, then capture the generated HTML with a browser.
"""
from pathlib import Path
from html import escape
import json

ROOT=Path(__file__).resolve().parents[1]
DEST=ROOT/'docs/screenshots'
DEST.mkdir(exist_ok=True)
logs=ROOT/'docs/evidence/protected'

def lines(name): return (logs/(name+'.log')).read_text().splitlines()
def pick(name,conditions):
    return '\n'.join(line for line in lines(name) if any(s in line for s in conditions))
def span(name,start,end):
    text=(logs/(name+'.log')).read_text(); a=text.index(start); b=text.index(end,a)+len(end)
    # Keep the complete last line rather than clipping the literal log contents.
    b=text.find('\n',b)
    return text[a:] if b<0 else text[a:b]

figs=[
 ('01-lobby', 'Lobby and private role assignment',[
  ('Moderator output (server.log, selected lines)',pick('server',['Starting a 9-player lobby.','Full 9 listening','#1 Player1 joined','#9 Player9 joined','Game started.','  #1 Player1: Sheriff','  #6 Player6: Godfather','Current scene: Role Reveal'])),
  ('Player 1 output (player1.log, selected lines)',pick('player1',['You are #1 Player1.','You are the Sheriff','Day 1 introductions']))]),
 ('02-night','Night actions and private delivery',[
  ('Mafia player 6 (player6.log, selected lines)',pick('player6',['You are the Godfather','protected-night-probe','Night choice set to #1'])),
  ('Town players 1, 2, and 3 (separate logs, selected lines)', '\n'.join([
   pick('player1',['Night 1 has begun.','Night 1: #7 appears suspicious','You were attacked and protected','Nobody died during the night.']).split('Nobody died during the night.')[0]+'Nobody died during the night.',
   pick('player2',['Your protected target #1 was attacked']),pick('player3',['visitors to #1'])]))]),
 ('03-trial','Discussion, nomination, and defense',[
  ('Player 1 (player1.log, continuous excerpt)',span('player1','[PUBLIC_SYSTEM] [phase 5','[PUBLIC] #8 Player8: I claim to be Town.'))]),
 ('04-execution','Judgment, last words, and will',[
  ('Player 1 (player1.log, continuous excerpt)',span('player1','[PUBLIC_SYSTEM] [phase 8','[PUBLIC_SYSTEM] #8 Player8 was publicly executed.'))]),
 ('05-result','Night death and a completed protected match',[
  ('Player 1 (player1.log, selected lines)',pick('player1',['#4 Player4 died.','#6 Player6 was publicly executed.','#7 Player7 was publicly executed.','Final faction result:','Postgame: everyone','Good game from the dead player.','[CLOSED] host shutdown']))]),
 ('06-cleanup','Protected startup and shutdown restoration',[
  ('Moderator output (server.log, selected startup lines)',pick('server',['Applying temporary','Verified player isolation: {"uid": 2001','Verified player isolation: {"uid": 2009'])),
  ('Moderator output (server.log, final cleanup lines)',pick('server',['Removed temporary player account: fearless_p1','Removed temporary player account: fearless_p9','Restored previous /proc settings']))]),
]
manifest=[]
for name,title,panels in figs:
 sections=''.join('<section><h2>'+escape(label)+'</h2><pre>'+escape(text)+'</pre></section>' for label,text in panels)
 html='''<!doctype html><html><meta charset="utf-8"><style>
 *{box-sizing:border-box}body{margin:0;padding:26px;background:white;color:#171717;width:1200px;font-family:Arial,sans-serif}
 h1{font-size:23px;margin:0 0 8px}p{font-size:15px;margin:0 0 18px;color:#444}
 section{margin:14px 0 0;border:1px solid #999}h2{font-size:15px;margin:0;padding:9px 12px;background:#eee;font-weight:normal}
 pre{font-family:'DejaVu Sans Mono',monospace;font-size:17px;line-height:1.45;white-space:pre-wrap;overflow-wrap:anywhere;margin:0;padding:14px;background:#111;color:#f5f5f5}
 </style><body><h1>'''+escape(title)+'''</h1><p>Captured game output | November 12, 2026 | Disposable protected VM | Transcript viewer</p>'''+sections+'''<p style="margin-top:15px">Excerpts retain the original output. Full logs: evidence/protected/. Scripted players use separate Linux accounts.</p></body></html>'''
 path=DEST/(name+'.html');path.write_text(html)
 manifest.append({'html':str(path),'png':str(DEST/(name+'.png'))})
(DEST/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('Prepared',len(figs),'transcript-viewer pages from captured logs.')
