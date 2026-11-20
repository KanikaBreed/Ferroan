# SPDX-License-Identifier: GPL-3.0-or-later
# The pin RegionServer image ships libSkiaSharp.so. That library needs
# libfontconfig.so.1, which mcr.microsoft.com/dotnet/runtime does not include.
# VectorRenderModule loads SkiaSharp while the region starts
# (VectorRenderModule.cs Initialise). The pin Dockerfile is not modified.
FROM tranquillity-region:p0.11-pin
USER root
RUN apt-get update \
 && apt-get install -y --no-install-recommends libfontconfig1 \
 && rm -rf /var/lib/apt/lists/*
USER app
