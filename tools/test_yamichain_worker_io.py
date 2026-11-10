import pathlib,subprocess,sys,tempfile,unittest
from yamichain_worker_io import exchange

class WorkerIoTests(unittest.TestCase):
    def spawn(self,program):
        return subprocess.Popen([sys.executable,'-u','-c',program],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
    def test_repeated_commands_preserve_request_response_order(self):
        p=self.spawn('import sys,json\nfor s in sys.stdin: print(json.dumps(json.loads(s)),flush=True)')
        try:
            for value in ['Head',{'Query':[1,2,3]},'Head']:self.assertEqual(exchange(p,value),value)
        finally:p.terminate();p.wait();p.stdin.close();p.stdout.close()
    def test_hung_worker_is_terminated_and_existing_state_is_not_reset(self):
        with tempfile.TemporaryDirectory() as directory:
            state=pathlib.Path(directory)/'durable';state.write_bytes(b'existing signing and witness reservations')
            p=self.spawn('import time;time.sleep(100)')
            try:
                with self.assertRaises(TimeoutError):exchange(p,{'Commit':'retry same certificate'},timeout=.1)
                p.wait(timeout=3);self.assertEqual(state.read_bytes(),b'existing signing and witness reservations')
            finally:
                if p.poll() is None:p.kill();p.wait()
                p.stdin.close();p.stdout.close()

if __name__=='__main__':unittest.main()
