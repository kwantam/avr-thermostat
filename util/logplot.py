import matplotlib.pyplot as plt
from matplotlib.ticker import MultipleLocator
import subprocess
import sys

if len(sys.argv) < 2:
    print("USAGE: logplot.py <logfile>")
    sys.exit(1)

if len(sys.argv) > 2:
    lens = [int(arg) for arg in sys.argv[2:]]
else:
    # default: 24 hours, 6 hours, 2 hours
    lens = [3600 * l for l in [24, 6, 2]]

# longest plot is 24 hours
maxlen = max(lens)

file = sys.argv[1]
with subprocess.Popen(["/usr/bin/tail", "-n", str(maxlen), file], stdout=subprocess.PIPE, encoding="UTF8") as proc:
    if proc.stdout is None:
        print("ERR Popen did not produce stdout as requested")
        sys.exit(2)
    lines = proc.stdout.readlines()

# process log file
data = []
for line in lines:
    (num, rest) = line.split(' ', 1)

    # print any line that isn't just a normal status line
    if not rest.startswith("OK"):
        print(line)
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
        rec[k] = v

    data.append(rec)

# print the last line of the logfile
print(lines[-1])

temps = [(d['temp'] / 16) * (9 / 5) + 32 for d in data]
cmds = [d['cool'] for d in data]
times = [t/60 for t in range(len(temps))]

# plot the requested data
for start in lens:
    fig, ax = plt.subplots()
    twin1 = ax.twinx()

    p1 = ax.plot(times[-start:], temps[-start:], "C0", label="temperature")
    p2 = twin1.plot(times[-start:], cmds[-start:], "C1", label="cooling", linewidth=0.75, linestyle="--")

    ax.yaxis.set_major_locator(MultipleLocator(5))
    ax.yaxis.set_minor_locator(MultipleLocator(1))
    ax.grid(True, which='major', axis='y', color='gray', linewidth=0.5)
    ax.grid(True, which='minor', axis='y', color='gray', linewidth=0.5, linestyle=':')

    plt.savefig("fridge_%d.svg" % start)
