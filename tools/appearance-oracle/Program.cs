// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Pin9 AppearanceServerHandler on a loopback port, over copied SQLite stores.

if (args.Length == 5 && args[0] == "service" && ushort.TryParse(args[3], out ushort port) && port != 0)
{
    try
    {
        return ServiceCases.Run(args[1], args[2], port, args[4]);
    }
    catch (Exception error)
    {
        Console.Error.WriteLine(error.Message);
        return 1;
    }
}

Console.Error.WriteLine("usage: AppearanceOracle service generate|verify <pin> <port> <fixture>");
return 2;
