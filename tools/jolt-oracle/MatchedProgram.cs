// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Matched-native drop. Reuses the P3.57 recorder. Does not link ferroan-physics.

using System.Globalization;
using Ferroan.JoltConfigOracle;
using OpenSim.Region.PhysicsModules.Jolt.Harness;

namespace Ferroan.JoltOracle;

static class Program
{
    static int Main(string[] args)
    {
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        CultureInfo.CurrentUICulture = CultureInfo.InvariantCulture;
        Environment.SetEnvironmentVariable(HarnessOptions.PhysicsRateVariable, null);
        if (args.Length != 6
            || args[2] != "--revision" || args[4] != "--reference"
            || (args[0] != "generate" && args[0] != "verify"))
        {
            Console.Error.WriteLine("usage: JoltMatchedOracle generate|verify <directory> --revision <40 hex> --reference <path>");
            return 2;
        }
        string directory = Path.GetFullPath(args[1]);
        string revision = args[3];
        string reference = Path.GetFullPath(args[5]);
        if (revision.Length != 40 || revision.Any(ch => !Uri.IsHexDigit(ch)))
        {
            Console.Error.WriteLine("revision must be 40 hex characters");
            return 1;
        }
        try
        {
            if (args[0] == "generate")
                RecordOracle.GenerateMatched(directory, revision, reference);
            else
                RecordOracle.VerifyMatched(directory, revision, reference);
        }
        catch (NativeSkip skip)
        {
            Console.Error.WriteLine("SKIPPED: " + skip.Message);
            return 3;
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine(exception);
            return 1;
        }
        return 0;
    }
}
