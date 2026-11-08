import matplotlib.pyplot as plt
from matplotlib.ticker import MultipleLocator
import subprocess
import sys

if len(sys.argv) < 2:
    print("USAGE: logplot.py <logfile>")
    sys.exit(1)

file = sys.argv[1]
with subprocess.Popen(["/usr/bin/tail", "-n", "7200", file], stdout=subprocess.PIPE, encoding="UTF8") as proc:
    if proc.stdout is None:
        print("ERR Popen did not produce stdout as requested")
        sys.exit(2)
    lines = proc.stdout.readlines()

# process log file
data = []
for line in lines:
    (num, rest) = line.split(' ', 1)
    if not rest.startswith("OK"):
        print("WARN:", line)
        continue

    (_, rest) = rest.split(' ', 1)
    rec = {}
    while not len(rest) == 0:
        if rest.find(' ') != -1:
            (d, rest) = rest.split(' ', 1)
        else:
            (d, rest) = (rest, '')
        (k, v) = d.split('=', 1)
        v = int(v, 16)
        if k == "temp":
            # convert temperature to fahrenheit
            v = (v / 16) * (9 / 5) + 32
        rec[k] = v

    data.append(rec)

temps = [d['temp'] for d in data]
cmds = [d['cool'] for d in data]
times = [t/60 for t in range(len(temps))]

fig, ax = plt.subplots()
twin1 = ax.twinx()

p1 = ax.plot(times, temps, "C0", label="temperature")
p2 = twin1.plot(times, cmds, "C1", label="cooling", linewidth=1, linestyle="--")

ax.yaxis.set_major_locator(MultipleLocator(5))
ax.yaxis.set_minor_locator(MultipleLocator(1))
ax.grid(True, which='major', axis='y', color='gray', linewidth=0.5)
ax.grid(True, which='minor', axis='y', color='gray', linewidth=0.5, linestyle=':')

plt.savefig(sys.stdout.buffer, format="svg")
