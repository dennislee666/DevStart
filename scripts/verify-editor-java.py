#!/usr/bin/env python3
"""Verify editor save, compiler failure isolation and rollback with a disposable Java service."""
from pathlib import Path
import json,os,subprocess,tempfile,time,sys,shutil
repository=Path(__file__).resolve().parent.parent
binary=str(Path(sys.argv[1] if len(sys.argv)>1 else repository/'target/release/dev-start').resolve())
jdk=Path(os.environ['JAVA_HOME']) if os.environ.get('JAVA_HOME') else (Path(subprocess.check_output(['/usr/libexec/java_home'],text=True).strip()) if sys.platform=='darwin' else Path(shutil.which('javac') or '/missing-javac').resolve().parent.parent)
with tempfile.TemporaryDirectory(prefix='devstart-java-editor-',dir='/tmp') as temp:
 root=Path(temp).resolve();project=root/'project';project.mkdir();source=project/'Main.java';classes=project/'classes';classes.mkdir()
 original='package editorfixture; public class Main { public static void main(String[] args) throws Exception { System.out.println("JAVA_ORIGINAL"); while(true) Thread.sleep(100); } }\n'
 source.write_text(original)
 subprocess.run([str(jdk/'bin/javac'),'-proc:none','-d',str(classes),str(source)],check=True,capture_output=True)
 (project/'.dev-start.json').write_text(json.dumps({'version':1,'launches':[{'name':'Main','file':'Main.java','description':'disposable editor test','entry':{'Java':{'program':str(jdk/'bin/java'),'main_class':'editorfixture.Main','vm_args':[],'class_paths':['classes'],'enabled':True}}}]}))
 env=os.environ|{'DEV_START_DATA_DIR':str(root/'data')};out=open(root/'service.log','wb');server=subprocess.Popen([binary,'--serve'],env=env,stdout=out,stderr=out)
 def rpc(m,p={}):
  raw=subprocess.run([binary,'--api',m,json.dumps(p)],env=env,capture_output=True,text=True,check=True);v=json.loads(raw.stdout);assert 'error' not in v,v;return v['result']
 def until(m,p,predicate):
  end=time.monotonic()+18
  while True:
   v=rpc(m,p)
   if predicate(v):return v
   assert time.monotonic()<end,(m,v)
   time.sleep(.1)
 def ready(rid,marker):
  until('logs.search',{'run_id':rid,'keyword':marker},lambda r:r['matched_lines']>0)
 try:
  end=time.monotonic()+10
  while not (root/'data/api/control.sock').exists():assert time.monotonic()<end;time.sleep(.05)
  p=rpc('projects.create',{'path':str(project)});sid=p['scripts'][0]['id']
  doc=rpc('editor.open',{'script_id':sid});baseline=rpc('editor.versions',{'script_id':sid})[0]['id']
  first=rpc('runs.start',{'script_id':sid})['run_id'];ready(first,'JAVA_ORIGINAL');pid=rpc('runs.get',{'id':first})['pid']
  def save(text):
   global doc
   doc=rpc('editor.change',{'script_id':sid,'expected_revision':doc['revision'],'content':text})
   doc=rpc('editor.save',{'script_id':sid,'expected_revision':doc['revision']})
  save('this is not valid Java\n')
  failed=rpc('runs.restart',{'script_id':sid})['run_id']
  until('runs.get',{'id':failed},lambda r:r['state']=='failed')
  old=rpc('runs.get',{'id':first});assert old['active'] and old['pid']==pid
  failure=rpc('runs.get',{'id':failed});assert old['tab_open'] and not old['superseded'] and not failure['tab_open']
  assert old['console_id']==failure['console_id']
  assert len([r for r in rpc('runs.list') if r['tab_open']])==1
  doc=rpc('editor.restore',{'script_id':sid,'expected_revision':doc['revision'],'version_id':baseline})
  second=rpc('runs.restart',{'script_id':sid})['run_id'];ready(second,'JAVA_ORIGINAL')
  save(original.replace('JAVA_ORIGINAL','JAVA_UPDATED'))
  third=rpc('runs.restart',{'script_id':sid})['run_id'];ready(third,'JAVA_UPDATED')
  doc=rpc('editor.restore',{'script_id':sid,'expected_revision':doc['revision'],'version_id':baseline})
  fourth=rpc('runs.restart',{'script_id':sid})['run_id'];ready(fourth,'JAVA_ORIGINAL')
  assert rpc('runs.get',{'id':fourth})['console_id']==old['console_id']
  assert len([r for r in rpc('runs.list') if r['tab_open']])==1
  report={'result':'passed','compiler':str(jdk/'bin/javac'),'invalid_source_keeps_original_service':True,'edited_source_changes_actual_output':True,'rollback_changes_next_actual_output':True,'versions':len(rpc('editor.versions',{'script_id':sid})),'scope':'isolated disposable Java source; no business project, Maven or deployment commands'}
  (repository/'docs/editor-java-integration.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n');print(json.dumps(report,ensure_ascii=False))
 finally:
  try:rpc('runs.stop_all')
  except Exception:pass
  server.terminate();server.wait(timeout=15);out.close()
