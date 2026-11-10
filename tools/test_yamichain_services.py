import unittest
import os,pathlib,signal,subprocess,sys,time
from yamichain_services import Services

class Process:
    pid=1
    def __init__(self):self.code=None
    def poll(self):return self.code

class SupervisionTests(unittest.TestCase):
    def test_restarts_back_off_and_live_processes_are_not_replaced(self):
        now=[0];created=[]
        def factory():
            p=Process();created.append(p);return p
        manager=Services(lambda:now[0]);manager.add('witness',factory)
        manager.tick();self.assertEqual(len(created),1)
        created[-1].code=1;manager.tick();self.assertEqual(len(created),2)
        created[-1].code=1;manager.tick();self.assertEqual(len(created),2)
        now[0]=1;manager.tick();self.assertEqual(len(created),3)
        now[0]=62;manager.tick();self.assertEqual(manager.services['witness']['delay'],1)
        self.assertTrue(manager.status()['witness']['running'])
        self.assertEqual(manager.status()['witness']['restarts'],2)
    def test_spawn_failure_is_bounded_and_does_not_replace_the_factory_or_store(self):
        now=[0];p=Process();calls=[]
        def factory():
            calls.append(1)
            if len(calls)>1:raise OSError('not logged')
            return p
        manager=Services(lambda:now[0]);manager.add('validator',factory);p.code=-9
        for _ in range(100):manager.tick()
        self.assertEqual(len(calls),2)
        self.assertTrue(manager.status()['validator']['spawn_failed'])
        now[0]=1;manager.tick();self.assertEqual(len(calls),3)
        with self.assertRaises(ValueError):manager.add('validator',factory)

class ParentDeathTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform=='linux','Linux parent-death contract')
    def test_wrapper_kill_terminates_initialized_child(self):
        helper=str(pathlib.Path(__file__).with_name('yamichain_child.py'))
        payload="import time;print('ready',flush=True);time.sleep(100)"
        program=("import os,subprocess,sys,time; p=subprocess.Popen([sys.executable,"+repr(helper)+",str(os.getpid()),sys.executable,'-u','-c',"+repr(payload)+"],stdout=subprocess.PIPE,text=True); assert p.stdout.readline().strip()=='ready'; print(p.pid,flush=True); time.sleep(100)")
        parent=subprocess.Popen([sys.executable,'-u','-c',program],stdout=subprocess.PIPE,text=True)
        child=int(parent.stdout.readline())
        try:
            parent.kill();parent.wait(timeout=3)
            end=time.monotonic()+3
            while time.monotonic()<end:
                path=pathlib.Path('/proc')/str(child)/'stat'
                if not path.exists() or path.read_text().split(') ',1)[1].split()[0]=='Z':break
                time.sleep(.05)
            else:self.fail('supervised child remained live after parent death')
        finally:
            if parent.poll() is None:parent.kill();parent.wait()
            parent.stdout.close()
            try:os.kill(child,signal.SIGTERM)
            except ProcessLookupError:pass

if __name__=='__main__':unittest.main()
