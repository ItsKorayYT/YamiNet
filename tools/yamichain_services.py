"""Process supervision only. Stores are never initialized, reset or replaced here."""
import time


class Services:
    def __init__(self, clock=time.monotonic):
        self.clock = clock
        self.services = {}

    def add(self, name, factory):
        if name in self.services:
            raise ValueError('duplicate managed service')
        process = factory()
        self.services[name] = dict(factory=factory, process=process, started=self.clock(),
                                   due=0, delay=1, restarts=0, spawn_failed=False)
        return process

    def tick(self):
        now = self.clock()
        for service in self.services.values():
            process = service['process']
            if process is not None and process.poll() is None:
                if now - service['started'] >= 60:
                    service['delay'] = 1
                continue
            if now < service['due']:
                continue
            service['due'] = now + service['delay']
            service['delay'] = min(60, service['delay'] * 2)
            try:
                service['process'] = service['factory']()
                service['started'] = now
                service['restarts'] += 1
                service['spawn_failed'] = False
            except OSError:
                service['process'] = None
                service['spawn_failed'] = True

    def status(self):
        return {name: dict(pid=s['process'].pid if s['process'] else None,
                           running=s['process'] is not None and s['process'].poll() is None,
                           restarts=s['restarts'], spawn_failed=s['spawn_failed'])
                for name, s in self.services.items()}
