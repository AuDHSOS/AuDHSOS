# membench

What the memory of the development machine does: the bandwidth of a
sequential stream and the latency of a random walk. Run it through the
xtask, which builds it with optimizations — a benchmark from the `dev`
profile measures the bounds checks, not the machine:

```
sh tools/xtask.sh membench [--size <mebibytes>] [--passes <count>] [--steps <count>]
```

The numbers describe the host, not the system this repository builds.
