// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Pin8 reads a row Ferroan wrote, on MySQL and PostgreSQL.
// The connection string arrives in FERROAN_ZONE_ADO and is not printed.

if (args is ["zone", ..])
    return ZoneRun.Run(args);
return 2;
