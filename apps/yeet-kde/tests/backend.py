#!/usr/bin/python3
"""Native UI test fixture only. Never contacts a receiver."""
import json, sys, threading
from pathlib import Path
lock = threading.Lock()
session, position, phase, timer = 0, 0, "playing", None
def emit(value):
    with lock:
        print(json.dumps(value), flush=True)
def state(phase_value=None, fraction=None):
    emit({"event":"state","session_id":session,"state":{
        "phase":phase_value or phase,"position_seconds":position,"duration_seconds":120,
        "preparation_operation":"Transcoding","preparation_fraction":fraction,
        "message":"Preparing video" if fraction else None,"notices":[]}})
for line in sys.stdin:
    request=json.loads(line)
    with open(__file__+".log","a") as log:
        log.write(json.dumps(request)+"\n")
    method, params, result = request["method"], request.get("params",{}), {}
    if method=="hello":
        result={"version":1}
    elif method=="inspect":
        result={"media":{"path":params["file"],"duration_seconds":120},"resume_position":25,
            "subtitles":[
                {"kind":"embedded","index":2,"codec":"subrip","language":"eng","title":"English","supported":True},
                {"kind":"embedded","index":3,"codec":"subrip","language":"dut","title":"Dutch","supported":True},
                {"kind":"external","path":str(Path(params["file"]).with_suffix(".srt")),"supported":True}]}
    elif method=="discover":
        result={"devices":[
            {"id":"speaker","name":"Speaker","model":"Audio receiver","capabilities":4,"addresses":["127.0.0.1"]},
            {"id":"tv","name":"Test TV","model":"Video receiver","capabilities":1,"addresses":["127.0.0.1"]}]}
    elif method=="start":
        session+=1
        position, phase = params["position"], "playing"
        emit({"id":request["id"],"ok":True,"result":{"session_id":session}})
        state("preparing",0.42)
        timer=threading.Timer(0.8,state)
        timer.daemon=True
        timer.start()
        continue
    elif method in ("pause","play","seek"):
        if method=="pause": phase="paused"
        elif method=="play": phase="playing"
        else: position=params["position"]
        emit({"id":request["id"],"ok":True,"result":{}})
        state()
        continue
    elif method=="stop":
        if timer: timer.cancel()
        emit({"id":request["id"],"ok":True,"result":{}})
        emit({"event":"ended","session_id":session,"phase":"cancelled","error":None})
        state("playing") # Late event must not reopen a stopped session.
        continue
    elif method=="shutdown":
        if timer: timer.cancel()
        emit({"id":request["id"],"ok":True,"result":{}})
        break
    emit({"id":request["id"],"ok":True,"result":result})
