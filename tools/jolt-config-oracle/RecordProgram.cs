// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Globalization;
using OpenSim.Region.PhysicsModules.Jolt.Harness;

namespace Ferroan.JoltConfigOracle;

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
            Console.Error.WriteLine("usage: JoltRecordOracle generate|verify <directory> --revision <40 hex> --reference <path>");
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
                RecordOracle.Generate(directory, revision, reference);
            else
                RecordOracle.Verify(directory, revision, reference);
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
