import json
import requests
import jsonschema
import yaml
from dateutil import parser
import urllib3

def run():
    requests.post("u", data=1)
    jsonschema.validate(1, 2)
    yaml.safe_load("x")
    parser.parse("2020")
    json.loads("x")
    http = urllib3.PoolManager()
    http.request("GET", "u")
