# Example compose grid

`compose.yaml` is four servers on the host network: Tranquillity's GridServer
and a Tranquillity region, built from the pin's Dockerfiles, plus
`ferroan-robust` (assets) and `ferroan-region`. Host networking is required
because the harness INI files bind `127.0.0.1` and the neighbouring loopback
addresses.

The region tag is that pin image plus `libfontconfig1`. `libSkiaSharp.so` needs it, and `VectorRenderModule` loads SkiaSharp while the region starts. The pin Dockerfile is not modified.

The C# services use the image entrypoints
`Source/OpenSim.Server.GridServer/AppData/Scripts/grid-start.sh` and
`Source/OpenSim.Server.RegionServer/AppData/Scripts/region-start.sh`. Mount
INI files under `/app/config`. `REGIONDIR` for the neighbour is an empty
directory so that script does not load a second `Regions.ini`; the region
file's `regionload_regionsdir` points at the real region directory. Data and
logs are `/app/data` and `/app/logdata`.

`cargo xtask docker build` builds the images and does not push them.
`cargo xtask docker smoke` builds them, checks the default user is not root,
restarts `ferroan-robust` against a mounted asset database, then logs in with
`ferroan-bot` and Ferrobot. Smoke runs the processes with `docker run` on the
same images. On a rootful daemon that is the host user. On a rootless daemon
it is container uid 0, which is that same user; a subordinate uid cannot read
the mounts. The mode-0600 INI files stay unreadable by other host accounts.
Smoke writes `RUN_UID` and `RUN_GID` the same way, plus generated passwords,
to `deploy/compose/.env`, which git ignores. Do not reuse a `.env` that has
`RUN_UID=0` against a rootful daemon. On that daemon container uid 0 is host
root. Local chat, instant messages, and Firestorm are not claimed.

Publishing to a registry is not part of this layout.
