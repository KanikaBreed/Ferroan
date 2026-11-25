// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Text.Json;
using OpenMetaverse;
using OpenSim.Data;
using OpenSim.Data.Null;

if (args.Length == 3 && args[0] == "mysql")
    return MysqlCross.Run(args[1], args[2]);

if (args.Length != 4 || args[0] is not ("generate" or "verify") || args[2] != "--revision")
    return 2;

// Each step includes its arguments so Rust replays the same calls and field values.
NullPresenceData[] stores = [new("", "first"), new("", "second")];
List<object> steps = [];
PresenceData held = null;
UUID Id(int n) => UUID.Parse($"aaaaaaaa-aaaa-4000-8000-{n:x12}");
var a = Id(1);
var b = Id(2);
var c = Id(3);
string user = Id(4).ToString();
var region = Id(5);
var secure = Id(6);

object Row(PresenceData row) => row == null ? null : new {
    user = row.UserID, session = row.SessionID.ToString(), region = row.RegionID.ToString(), data = row.Data
};
void Step(int instance, string op, object input, object result) =>
    steps.Add(new { instance, op, input, result });
void Store(int i, UUID session, string owner, string tag) {
    var row = new PresenceData { SessionID = session, RegionID = region, UserID = owner,
        Data = new() { ["SecureSessionID"] = secure.ToString(), ["Tag"] = tag, ["tag"] = "lower" } };
    Step(i, "store", Row(row), stores[i].Store(row));
}
void Get(int i, UUID session) {
    held = stores[i].Get(session);
    Step(i, "get", session.ToString(), Row(held));
}
void Find(int i, string field, string value) =>
    Step(i, "find", new { field, value }, stores[i].Get(field, value).Select(Row).ToArray());
void Delete(int i, string field, string value) =>
    Step(i, "delete", new { field, value }, stores[i].Delete(field, value));
void Report(int i, UUID session, UUID to) =>
    Step(i, "report", new { session = session.ToString(), region = to.ToString() }, stores[i].ReportAgent(session, to));
void Logout(int i, UUID from) {
    stores[i].LogoutRegionAgents(from);
    Step(i, "logout", from.ToString(), null);
}
void Verify(int i, UUID who, UUID key) =>
    Step(i, "verify", new { user = who.ToString(), secure = key.ToString() }, stores[i].VerifyAgent(who, key));

Get(0, a);
Report(1, a, region);
Delete(0, "SessionID", a.ToString());
Store(0, a, user, "blue");
Store(1, b, user, "blue");
Get(1, a);
Report(0, a, UUID.Zero);
Step(1, "held", null, Row(held));
Verify(0, Id(4), secure);
Verify(1, Id(7), secure);
Find(0, "RegionID", UUID.Zero.ToString());
Find(1, "UserID", user);
Find(0, "UserID", user.ToUpperInvariant());
Find(1, "SessionID", " " + a.ToString().ToUpperInvariant() + " ");
Find(1, "SessionID", a.ToString().Replace("-", ""));
Find(1, "SessionID", "{" + a + "}");
Find(1, "RegionID", "bad-region");
Find(0, "SecureSessionID", secure.ToString());
Find(0, "Tag", "blue");
Find(0, "tag", "blue");
Find(0, "absent", "");
Store(1, a, "hypergrid@example.invalid", "red");
Step(0, "held", null, Row(held));
Get(0, a);
Find(0, "UserID", user);
Delete(0, "RegionID", "bad-region");
Delete(0, "SessionID", "bad-session");
Delete(1, "Tag", "red");
Store(0, c, user, "blue");
Find(1, "UserID", user);
Delete(0, "SessionID", c.ToString().ToUpperInvariant());
Store(1, c, user, "blue");
Report(1, b, UUID.Zero);
Logout(0, region);
Get(1, c);
Get(1, b);
Delete(0, "RegionID", UUID.Zero.ToString());
Get(1, b);
Store(0, a, user, "same");
Store(1, b, user, "same");
Delete(1, "UserID", user);
Find(0, "Tag", "same");
Delete(0, "Tag", "same");

// Reuse two freed dictionary slots. A field lookup exposes their order.
Store(0, a, user, "drop");
Store(1, b, user, "keep");
Store(0, c, user, "drop");
Delete(1, "Tag", "drop");
Store(1, Id(8), user, "new");
Store(0, Id(9), user, "new");
Find(1, "UserID", user);
Get(0, b);
Report(1, b, Id(10));
Step(0, "held", null, Row(held));
Step(1, "restore", null, stores[1].Store(held));
Report(0, Id(9), UUID.Zero);
Logout(1, UUID.Zero);
Find(0, "RegionID", Id(10).ToString());
Get(0, Id(9));
Logout(0, region);
Find(1, "UserID", user);
Logout(1, Id(10));
Find(0, "UserID", user);

string json = JsonSerializer.Serialize(new { revision = args[3], steps },
    new JsonSerializerOptions { WriteIndented = true, NewLine = "\n" }) + "\n";
string path = Path.Combine(args[1], "null.json");
if (args[0] == "generate") {
    Directory.CreateDirectory(args[1]);
    File.WriteAllText(path, json);
} else if (File.ReadAllText(path) != json) {
    Console.Error.WriteLine("Presence fixture differs");
    return 1;
}
return 0;
