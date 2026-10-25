#!/usr/bin/env python3
"""Write types.bag with ROS Noetic's rosbag, then read it back with ROS's own deserializer into types.expected.json.

Run in ros:noetic-ros-base after:
  apt-get install -y --no-install-recommends ros-noetic-roslz4 ros-noetic-std-msgs ros-noetic-sensor-msgs \
    ros-noetic-geometry-msgs ros-noetic-diagnostic-msgs ros-noetic-tf2-msgs ros-noetic-visualization-msgs \
    ros-noetic-rosgraph-msgs
  source /opt/ros/noetic/setup.bash
  python3 generate.py <output dir>
"""
import hashlib, json, sys
from pathlib import Path

import genpy, rosbag, rospy
from genpy.dynamic import generate_dynamic
from std_msgs.msg import Header, String, Empty, ColorRGBA
from geometry_msgs.msg import Point, Quaternion, Vector3, Transform, TransformStamped
from sensor_msgs.msg import Imu, JointState, NavSatFix, NavSatStatus, Image
from tf2_msgs.msg import TFMessage
from diagnostic_msgs.msg import DiagnosticArray, DiagnosticStatus, KeyValue
from visualization_msgs.msg import Marker
from rosgraph_msgs.msg import Log

SEP = "=" * 80
CUSTOM = """Header header
int8 NEG=-3
string TEXT=a # not a comment = still the value
bool flag
byte b
char c
int16 i16
uint16 u16
int32 i32
uint32 u32
int64 i64
uint64 u64
float32 f32
float64 f64
string s
time t
duration d
Inner inner
Inner[] inners
Inner[2] pair
uint8[] blob
uint8[4] quad
string[] names
string[2] two_names
time[] times
duration[2] durations
bool[] flags
geometry_msgs/Point[] points
std_msgs/Empty nothing
""" + f"""{SEP}
MSG: std_msgs/Header
uint32 seq
time stamp
string frame_id
{SEP}
MSG: rosbag1_test/Inner
float64 x
string label
int32[] values
{SEP}
MSG: geometry_msgs/Point
float64 x
float64 y
float64 z
{SEP}
MSG: std_msgs/Empty
"""

T = rospy.Time
D = rospy.Duration


def hdr(seq, s, ns, frame):
    return Header(seq=seq, stamp=T(s, ns), frame_id=frame)


def messages():
    yield "/imu", Imu(header=hdr(7, 1700000000, 123456789, "imu_link"),
                      orientation=Quaternion(0.0, 0.0, 0.7071067811865476, 0.7071067811865476),
                      orientation_covariance=[0.01 * i for i in range(9)],
                      angular_velocity=Vector3(0.1, -0.2, 0.3), angular_velocity_covariance=[0.0] * 9,
                      linear_acceleration=Vector3(0.0, 0.0, 9.80665), linear_acceleration_covariance=[-1.0] + [0.0] * 8)
    yield "/joints", JointState(header=hdr(1, 1700000000, 5, ""), name=["shoulder", "elbow"],
                                position=[0.5, -1.25], velocity=[0.0, 0.1], effort=[])
    yield "/joints", JointState(header=hdr(2, 1700000001, 5, ""))
    tf = lambda i: TransformStamped(header=hdr(i, 1700000002, i, "world"), child_frame_id=f"link{i}",
                                    transform=Transform(Vector3(i, 2.0 * i, -i), Quaternion(0, 0, 0, 1)))
    yield "/tf", TFMessage(transforms=[tf(1), tf(2)])
    yield "/diagnostics", DiagnosticArray(header=hdr(3, 1700000003, 0, ""), status=[
        DiagnosticStatus(level=DiagnosticStatus.WARN, name="battery", message="low", hardware_id="bms",
                         values=[KeyValue("voltage", "21.3"), KeyValue("cells", "6")]),
        DiagnosticStatus(level=DiagnosticStatus.OK, name="motors", message="", hardware_id="", values=[])])
    yield "/fix", NavSatFix(header=hdr(4, 1700000004, 999999999, "gps"),
                            status=NavSatStatus(status=NavSatStatus.STATUS_FIX, service=NavSatStatus.SERVICE_GPS),
                            latitude=48.8566, longitude=2.3522, altitude=35.0,
                            position_covariance=[1.0, 0, 0, 0, 1.0, 0, 0, 0, 4.0],
                            position_covariance_type=NavSatFix.COVARIANCE_TYPE_DIAGONAL_KNOWN)
    yield "/marker", Marker(header=hdr(5, 1700000005, 0, "map"), ns="path", id=-4, type=Marker.LINE_STRIP,
                            action=Marker.ADD, scale=Vector3(0.1, 0.0, 0.0), color=ColorRGBA(1.0, 0.5, 0.25, 1.0),
                            lifetime=D(2, 500000000), frame_locked=True, points=[Point(0, 0, 0), Point(1, 2, 3)],
                            colors=[ColorRGBA(0, 0, 0, 1), ColorRGBA(1, 1, 1, 0.5)], text="", mesh_resource="",
                            mesh_use_embedded_materials=False)
    for lvl, text in [(Log.WARN, "battery low"), (Log.ERROR, "lost link"), (Log.FATAL, "abort")]:
        yield "/rosout", Log(header=hdr(lvl, 1700000006, lvl, ""), level=lvl, name="/node", msg=text,
                             file="node.py", function="run", line=42, topics=["/rosout", "/cmd"])
    yield "/image", Image(header=hdr(6, 1700000007, 0, "cam"), height=2, width=2, encoding="rgb8",
                          is_bigendian=0, step=6, data=bytes(range(12)))
    yield "/text", String(data="héllo ✓")
    yield "/text", String(data="")
    yield "/empty", Empty()
    # Noetic's Python rosbag keeps one connection per topic: a second type on a topic would be
    # stored under the first connection, so two connections on one topic need a hand-built fixture.
    yield "/mixed", String(data="first")
    custom_types = generate_dynamic("rosbag1_test/Custom", CUSTOM)
    Custom, Inner = custom_types["rosbag1_test/Custom"], custom_types["rosbag1_test/Inner"]
    yield "/custom", Custom(header=hdr(4294967295, 4294967295, 999999999, "c"), flag=True, b=-128, c=255,
                            i16=-32768, u16=65535, i32=-2147483648, u32=4294967295,
                            i64=-9223372036854775808, u64=18446744073709551615, f32=0.1, f64=-1e-300,
                            s="ascii", t=T(1, 2), d=D(-1, 500000000),
                            inner=Inner(x=1.5, label="in", values=[1, -2, 3]),
                            inners=[Inner(x=0.0, label="", values=[]), Inner(x=-2.5, label="z", values=[9])],
                            pair=[Inner(x=1.0, label="a", values=[1]), Inner(x=2.0, label="b", values=[])],
                            blob=bytes([0, 1, 254, 255]), quad=bytes([9, 8, 7, 6]), names=["n1", "n2", "n3"],
                            two_names=["left", "right"], times=[T(0, 0), T(10, 20)], durations=[D(0, 1), D(-3, 0)],
                            flags=[True, False, True], points=[Point(1, 2, 3)], nothing=custom_types["std_msgs/Empty"]())


def to_json(v):
    if isinstance(v, genpy.Message):
        return {name: to_json(getattr(v, name)) for name in v.__slots__}
    if isinstance(v, (genpy.Time, genpy.Duration)):
        return v.secs * 10**9 + v.nsecs
    if isinstance(v, bytes):
        return list(v)
    if isinstance(v, (list, tuple)):
        return [to_json(x) for x in v]
    return v


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    path = out / "types.bag"
    with rosbag.Bag(str(path), "w", compression=rosbag.Compression.NONE, chunk_threshold=1024) as bag:
        for i, (topic, msg) in enumerate(messages()):
            bag.write(topic, msg, t=T(1700000100, i))
        # connection_header replaces the whole header, so it carries type, md5sum and definition too.
        bag.write("/latched", String(data="latched"), t=T(1700000200, 0),
                  connection_header={"callerid": "/talker", "latching": "1", "type": String._type,
                                     "md5sum": String._md5sum, "message_definition": String._full_text})
    expected = [{"topic": topic, "type": msg._type, "time_ns": t.secs * 10**9 + t.nsecs, "value": to_json(msg)}
                for topic, msg, t in rosbag.Bag(str(path)).read_messages()]
    (out / "types.expected.json").write_text(json.dumps(expected, indent=1, ensure_ascii=False, allow_nan=False) + "\n")
    for p in (path, out / "types.expected.json"):
        print(f"{p.name}: sha256={hashlib.sha256(p.read_bytes()).hexdigest()} bytes={p.stat().st_size}")


if __name__ == "__main__":
    main()
