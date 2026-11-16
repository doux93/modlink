#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
ModuLink AT 模组模拟器 —— 剧本驱动的蜂窝模组行为状态机。

用法:
    python at_sim.py --port COM11 [--scenario normal] [--baud 115200]

配合 com0com 虚拟串口对使用: 应用连 COM10, 模拟器挂 COM11。
运行中可在控制台输入剧本名切换 (模拟现场拔卡/弱信号等), 输入 quit 退出。

剧本:
    normal       一切正常, 延时注册成功
    no_sim       SIM 未插入 (CPIN 报 +CME ERROR: 10)
    pin_locked   SIM 设置了 PIN 码
    weak_signal  信号极弱 (CSQ 3±2), 注册慢、随机掉线 URC
    reg_denied   网络注册被拒绝 (CEREG: 0,3)
    apn_wrong    注册成功但 PDP 激活失败 (+CME ERROR: 148)
    flaky        随机 10% 指令返回 ERROR, 模拟接触不良
"""
import argparse
import random
import re
import sys
import threading
import time

try:
    import serial
except ImportError:
    print("需要 pyserial: pip install pyserial", file=sys.stderr)
    sys.exit(1)

SCENARIOS = ["normal", "no_sim", "pin_locked", "weak_signal", "reg_denied", "apn_wrong", "flaky"]

CME = {10: "SIM not inserted", 11: "SIM PIN required", 148: "unspecified GPRS error"}


class ModemState:
    def __init__(self, scenario: str):
        self.lock = threading.Lock()
        self.echo = True
        self.boot_ts = time.monotonic()
        self.sockets = {}          # id -> (type, addr, port)
        self.pdp_active = False
        self.cereg_urc_mode = 0    # AT+CEREG=n
        self.apn = ""
        self.set_scenario(scenario)

    def set_scenario(self, s: str):
        with self.lock:
            self.scenario = s
            self.boot_ts = time.monotonic()
            self.pdp_active = False
            self.sockets.clear()

    # ---- 派生状态 ----
    def cpin(self):
        if self.scenario == "no_sim":
            return None, 10
        if self.scenario == "pin_locked":
            return "SIM PIN", None
        return "READY", None

    def csq(self):
        if self.scenario == "no_sim":
            return 99
        if self.scenario == "weak_signal":
            return max(0, 3 + random.randint(-2, 2))
        # 正常: 22±3 缓慢游走, 供 CSQ 曲线演示
        base = 22 + int(3 * random.uniform(-1, 1))
        return min(31, max(10, base))

    def cereg_stat(self):
        """注册状态: 模拟真实的延时注册过程。"""
        if self.scenario in ("no_sim", "pin_locked"):
            return 0
        if self.scenario == "reg_denied":
            return 3
        elapsed = time.monotonic() - self.boot_ts
        wait = 25 if self.scenario == "weak_signal" else 6
        return 1 if elapsed > wait else 2


class AtSimulator:
    def __init__(self, port: str, baud: int, scenario: str):
        self.ser = serial.Serial(port, baud, timeout=0.05)
        self.st = ModemState(scenario)
        self.running = True
        self._urc_thread = threading.Thread(target=self._urc_loop, daemon=True)

    # ------------------------------------------------ IO
    def send_line(self, text: str):
        self.ser.write((text + "\r\n").encode("utf-8", "replace"))

    def reply(self, lines, final="OK", delay=0.05):
        time.sleep(delay)
        self.ser.write(b"\r\n")
        for ln in lines:
            self.send_line(ln)
        if final:
            self.send_line(final)

    def cme(self, code: int):
        self.reply([], final=f"+CME ERROR: {code}")

    # ------------------------------------------------ URC 后台线程
    def _urc_loop(self):
        last_stat = None
        while self.running:
            time.sleep(1.0)
            st = self.st
            # CEREG 状态变化 URC
            if st.cereg_urc_mode >= 1:
                cur = st.cereg_stat()
                if cur != last_stat:
                    last_stat = cur
                    self.send_line(f"+CEREG: {cur}")
            # 弱信号剧本: 随机 socket 掉线
            if st.scenario == "weak_signal" and st.sockets and random.random() < 0.05:
                sid = next(iter(st.sockets))
                del st.sockets[sid]
                self.send_line(f'+MIPURC: "disconn",{sid},1')

    # ------------------------------------------------ 指令处理
    def handle(self, raw: str):
        cmd = raw.strip()
        if not cmd:
            return
        if self.st.echo:
            self.ser.write((cmd + "\r\n").encode())

        if self.st.scenario == "flaky" and random.random() < 0.10:
            self.reply([], final="ERROR")
            return

        c = cmd.upper()

        # ---- 基础 ----
        if c == "AT":
            self.reply([])
        elif c in ("ATE0", "ATE1"):
            self.st.echo = c.endswith("1")
            self.reply([])
        elif c == "ATI" or c == "AT+CGMM":
            self.reply(["ML307R-DL"])
        elif c == "AT+CGMR":
            self.reply(["ML307R_V1.2.3_SIM"])
        elif c.startswith("AT+CGSN") or c == "AT+GSN":
            self.reply(["863505077804163"])
        elif c == "AT+ICCID":
            self.reply(["+ICCID: 89860436101880321550"])
        elif c == "AT+CIMI":
            self.reply(["460046545212900"])

        # ---- SIM / 信号 / 注册 ----
        elif c == "AT+CPIN?":
            state, err = self.st.cpin()
            if err:
                self.cme(err)
            else:
                self.reply([f"+CPIN: {state}"])
        elif c == "AT+CSQ":
            self.reply([f"+CSQ: {self.st.csq()},0"])
        elif c.startswith("AT+CEREG="):
            self.st.cereg_urc_mode = int(c.split("=")[1])
            self.reply([])
        elif c == "AT+CEREG?":
            stat = self.st.cereg_stat()
            extra = ',"1A2B","01F3C401",7' if stat == 1 else ""
            self.reply([f"+CEREG: {self.st.cereg_urc_mode},{stat}{extra}"])
        elif c == "AT+COPS?":
            if self.st.cereg_stat() == 1:
                self.reply(['+COPS: 0,0,"CHINA MOBILE",7'])
            else:
                self.reply(["+COPS: 0"])
        elif c == "AT+CEER":
            if self.st.scenario == "reg_denied":
                self.reply(["+CEER: EMM_CAUSE_15_NO_SUITABLE_CELLS"])
            else:
                self.reply(["+CEER: NO_ERROR"])

        # ---- PDP ----
        elif c.startswith("AT+CGDCONT="):
            m = re.match(r'AT\+CGDCONT=\d+,"IP.*?","?([^",]*)"?', c)
            self.st.apn = m.group(1) if m else ""
            self.reply([])
        elif c == "AT+CGDCONT?":
            apn = self.st.apn or ("" if self.st.scenario == "apn_wrong" else "cmiot")
            self.reply([f'+CGDCONT: 1,"IP","{apn}","0.0.0.0",0,0'])
        elif c.startswith("AT+CGACT=1"):
            if self.st.cereg_stat() != 1:
                self.cme(148)
            elif self.st.scenario == "apn_wrong" and self.st.apn.lower() not in ("cmiot", "cmnet"):
                time.sleep(1.5)
                self.cme(148)
            else:
                self.st.pdp_active = True
                self.reply([], delay=0.8)
        elif c.startswith("AT+CGPADDR"):
            ip = "10.123.45.67" if self.st.pdp_active else "0.0.0.0"
            self.reply([f'+CGPADDR: 1,"{ip}"'])

        # ---- DNS / Ping / Socket (ML30x 风格) ----
        elif c.startswith("AT+MDNSGIP="):
            if self.st.pdp_active:
                self.reply(['+MDNSGIP: "www.baidu.com","39.156.66.14"'], delay=0.5)
            else:
                self.cme(148)
        elif c.startswith("AT+MPING="):
            if self.st.pdp_active:
                self.reply(["+MPING: 39.156.66.14,52,89"], delay=0.9)
            else:
                self.cme(148)
        elif c.startswith("AT+MIPOPEN="):
            m = re.match(r"AT\+MIPOPEN=(\d+)", c)
            sid = int(m.group(1)) if m else 0
            if not self.st.pdp_active:
                self.cme(148)
            else:
                self.st.sockets[sid] = "TCP"
                delay = random.uniform(0.3, 2.5) if self.st.scenario == "weak_signal" else 0.4
                self.reply([], delay=delay)
                self.send_line(f"+MIPOPEN: {sid},0")
        elif c.startswith("AT+MIPSEND="):
            m = re.match(r"AT\+MIPSEND=(\d+)", c)
            sid = int(m.group(1)) if m else 0
            if sid in self.st.sockets:
                self.reply([f"+MIPSEND: {sid},0"])
            else:
                self.reply([], final="ERROR")
        elif c.startswith("AT+MIPCLOSE="):
            m = re.match(r"AT\+MIPCLOSE=(\d+)", c)
            sid = int(m.group(1)) if m else 0
            self.st.sockets.pop(sid, None)
            self.reply([f"+MIPCLOSE: {sid},0"])

        else:
            # 未知指令按真实模组习惯返回 ERROR
            self.reply([], final="ERROR")

    # ------------------------------------------------ 主循环
    def run(self):
        self._urc_thread.start()
        buf = b""
        print(f"[at_sim] 运行中 @ {self.ser.port}, 剧本: {self.st.scenario}")
        while self.running:
            data = self.ser.read(256)
            if not data:
                continue
            buf += data
            while True:
                m = re.search(rb"[\r\n]", buf)
                if not m:
                    break
                line, buf = buf[: m.start()], buf[m.end():]
                try:
                    self.handle(line.decode("utf-8", "replace"))
                except Exception as e:  # 模拟器自身不允许崩
                    print(f"[at_sim] 处理异常: {e}", file=sys.stderr)


def log_role_loop(port: str, baud: int):
    """日志口模式: 模拟模组 debug UART 的持续日志输出（配合设备组分屏演示）。"""
    ser = serial.Serial(port, baud)
    t0 = time.monotonic()
    lines = [
        "RRC: state={st}", "PHY: rsrp={rsrp} rsrq=-11 sinr={sinr}",
        "EMM: periodic TAU timer running", "MM: cell reselection, earfcn=3590",
        "SOC: tx_queue=0 rx_queue=0", "PDCP: ul_bytes={ub} dl_bytes={db}",
    ]
    states = ["IDLE", "CONNECTED", "CONNECTED", "CONNECTED"]
    print(f"[at_sim] 日志口模式 @ {port}, Ctrl+C 退出")
    ub = db = 0
    while True:
        time.sleep(random.uniform(0.15, 0.6))
        ub += random.randint(0, 400)
        db += random.randint(0, 1200)
        tpl = random.choice(lines)
        msg = tpl.format(
            st=random.choice(states), rsrp=random.randint(-105, -85),
            sinr=random.randint(5, 25), ub=ub, db=db,
        )
        ts = time.monotonic() - t0
        ser.write(f"[{ts:10.3f}] {msg}\r\n".encode())


def main():
    ap = argparse.ArgumentParser(description="ModuLink AT 模组模拟器")
    ap.add_argument("--port", required=True, help="串口名, 如 COM11")
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--scenario", default="normal", choices=SCENARIOS)
    ap.add_argument("--role", default="at", choices=["at", "log"],
                    help="at=AT口(默认); log=日志口(持续输出debug日志,配合设备组分屏)")
    args = ap.parse_args()

    if args.role == "log":
        try:
            log_role_loop(args.port, args.baud)
        except KeyboardInterrupt:
            pass
        return

    sim = AtSimulator(args.port, args.baud, args.scenario)
    t = threading.Thread(target=sim.run, daemon=True)
    t.start()

    # 控制台交互: 切换剧本
    print(f"可用剧本: {', '.join(SCENARIOS)}; 输入剧本名切换, quit 退出")
    for line in sys.stdin:
        s = line.strip()
        if s == "quit":
            sim.running = False
            break
        if s in SCENARIOS:
            sim.st.set_scenario(s)
            print(f"[at_sim] 剧本切换 -> {s}")
        elif s:
            print(f"未知剧本: {s}")


if __name__ == "__main__":
    main()
