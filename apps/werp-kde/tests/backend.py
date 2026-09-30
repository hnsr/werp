#!/usr/bin/python3
"""Native UI test fixture only. Never contacts a receiver."""
import json, os, sys, threading, tomllib
from pathlib import Path
lock = threading.Lock()
session, position, phase, timer = 0, 0, "playing", None
subtitle_changeable=True
def emit(value):
    with lock:
        print(json.dumps(value), flush=True)
def state(phase_value=None, fraction=None):
    emit({"event":"state","session_id":session,"state":{
        "phase":phase_value or phase,"position_seconds":position,"duration_seconds":120,
        "preparation_operation":"Transcoding","preparation_fraction":fraction,
        "message":"Preparing video" if fraction else None,"notices":[],"subtitles_changeable":subtitle_changeable}})
def conversion_formats(params):
    source={"container":"matroska","streams":[{"kind":"video","codec":"hevc","profile":"Main 10","width":1920,"height":1080},{"kind":"audio","codec":"ac3","channels":6}]}
    planned={"container":"mp4","streams":[{"kind":"video","codec":"h264","profile":"High"},{"kind":"audio","codec":"aac","profile":"LC","channels":2}]}
    if params.get("device_id")=="tv":
        planned["streams"][0]=dict(source["streams"][0])
    if "reuse" in params["file"] and params.get("device_id")=="tv":
        source["streams"][1]={"kind":"audio","codec":"aac","profile":"LC","channels":6}
        planned={"container":"mp4","streams":source["streams"]}
    if "wrapped" in params["file"]:
        source["streams"][0]["frame_rate"]=23.976
    return source,planned
for line in sys.stdin:
    request=json.loads(line)
    with open(__file__+".log","a") as log:
        log.write(json.dumps(request)+"\n")
    method, params, result = request["method"], request.get("params",{}), {}
    if method=="hello":
        result={"version":2}
    elif method=="inspect":
        result={"media":{"path":params["file"],"duration_seconds":120},"resume_position":25,"suggested_subtitles":{"kind":"embedded","index":2},
            "subtitles":[
                {"kind":"embedded","index":2,"codec":"subrip","language":"eng","title":"English","supported":True},
                {"kind":"embedded","index":3,"codec":"subrip","language":"dut","title":"Dutch","supported":True},
                {"kind":"external","path":str(Path(params["file"]).with_suffix(".srt")),"supported":True}]}
        if "bitmap" in params["file"]:
            result["subtitles"].append({"kind":"embedded","index":4,"codec":"hdmv_pgs_subtitle","supported":True,"burn_in":True})
    elif method=="get_subtitle_delay":
        result={"subtitle_delay_ms":0}
        custom=Path(__file__+".delays.json")
        if custom.exists():
            choice=params["subtitles"]
            result["subtitle_delay_ms"]=json.loads(custom.read_text()).get(str(choice.get("index")),0)
            delayed=threading.Timer(0.15,lambda reply={"id":request["id"],"ok":True,"result":result}: emit(reply))
            delayed.daemon=True; delayed.start()
            continue
    elif method=="discover":
        result={"devices":[
            {"id":"speaker","name":"Speaker","model":"Audio receiver","capabilities":4,"addresses":["127.0.0.1"]},
            {"id":"tv","name":"Test TV","model":"Video receiver","capabilities":1,"addresses":["127.0.0.1"]}]}
        custom=Path(__file__+".devices.json")
        if custom.exists():
            override=json.loads(custom.read_text())
            if "error" in override:
                emit({"id":request["id"],"ok":False,"error":{"message":override["error"]}})
                continue
            result=override
    elif method in ("get_gui_preferences","set_gui_last_device"):
        config=Path(params.get("path") or Path(os.environ.get("XDG_CONFIG_HOME",Path.home()/".config"))/"werp/config.toml")
        state_file=Path(os.environ["XDG_STATE_HOME"])/"werp/gui.toml"
        existing=tomllib.loads(state_file.read_text()) if state_file.exists() else {}
        result={"last_device_id":existing.get("last_device_id","")}
        if method=="set_gui_last_device":
            result["last_device_id"]=params["device_id"]
            state_file.parent.mkdir(parents=True,exist_ok=True)
            state_file.write_text(f'last_device_id = {json.dumps(result["last_device_id"])}\n')
        else:
            preferences=tomllib.loads(config.read_text()) if config.exists() else {}
            result["conversion_auto_close"]=preferences.get("gui",{}).get("conversion",{}).get("auto_close",True)
            result["warnings"]=[]
    elif method=="preview_conversion":
        source,planned=conversion_formats(params)
        result={"source":source,"planned_target":planned,"message":"Ready to convert","warnings":[]}
        if "slowpreview" in params["file"] and params.get("device_id")=="tv":
            preview_reply={"id":request["id"],"ok":True,"result":result}
            delayed=threading.Timer(0.25,lambda reply=preview_reply: emit(reply))
            delayed.daemon=True; delayed.start()
            continue
    elif method=="convert":
        session+=1
        source,planned=conversion_formats(params)
        conversion_state={"phase":"preparing","source":source,"planned_target":planned,"target_description":"MP4 · H.264 / HEVC · AAC-LC (up to 6 channels)","operation":"Converting to H.264 and stereo AAC in MP4","fraction":0.42,"message":"Converting video and audio","warnings":[]}
        emit({"id":request["id"],"ok":True,"result":{"operation_id":session}})
        emit({"event":"conversion_state","operation_id":session,"state":conversion_state})
        def converted(file=params["file"], operation=session, snapshot=conversion_state, output_format=planned):
            failed="fail" in file
            actual=json.loads(json.dumps(output_format))
            actual["container"]="mov,mp4,m4a,3gp,3g2,mj2"
            for stream in actual["streams"]:
                if stream["kind"]=="video":
                    stream.setdefault("width",1920); stream.setdefault("height",1080)
            final=dict(snapshot,phase="failed" if failed else "completed",fraction=1.0,
                output="/tmp/test.werp-prepared.mp4",message="Existing conversion reused" if "reuse" in file else "Conversion complete",reused="reuse" in file,
                error="Test conversion failure" if failed else None,
                target=actual)
            if "wrapped" in file:
                final["output"]="/tmp/downloads/Example.Series.S01E01.1080p.10bit.WEBRip.6CH.x265.HEVC/Example.Series.S01E01.1080p.10bit.WEBRip.6CH.x265.HEVC.werp-0123456789ab-cdef0123.mp4"
            emit({"event":"conversion_ended","operation_id":operation,"state":final})
        if "long" not in params["file"]:
            timer=threading.Timer(0.8,converted); timer.daemon=True; timer.start()
        continue
    elif method=="cancel_conversion":
        if timer: timer.cancel()
        emit({"id":request["id"],"ok":True,"result":{}})
        emit({"event":"conversion_ended","operation_id":session,"state":{"phase":"cancelled","message":"Cancelled; cleanup completed"}})
        emit({"event":"conversion_state","operation_id":session,"state":{"phase":"preparing","fraction":0.9}})
        continue
    elif method=="start":
        session+=1
        position, phase = params["position"], "playing"
        subtitle_changeable = params["subtitles"].get("index") != 4
        emit({"id":request["id"],"ok":True,"result":{"session_id":session}})
        state("preparing",0.42)
        timer=threading.Timer(0.8,state)
        timer.daemon=True
        timer.start()
        continue
    elif method=="set_subtitles":
        if params["subtitles"].get("kind")=="external" and "invalid" in params["subtitles"].get("path",""):
            emit({"id":request["id"],"ok":False,"error":{"message":"Invalid subtitle file"}}); continue
        emit({"id":request["id"],"ok":True,"result":{}}); state(); continue
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
