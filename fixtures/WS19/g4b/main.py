from factory import make
from svc import Service

def main():
    service = make()
    service.run()
