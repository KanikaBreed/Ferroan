// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Globalization;
using System.Text.Json;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Services.Interfaces;

static class AppearanceCases
{
    static UUID Id(int n) => UUID.Parse($"00000000-0000-4000-8000-{n:x12}");
    static string Pair(int n) => $"{Id(n)}:{Id(n + 100)}";

    static object Snapshot(AvatarAppearance a) => a == null ? null : new
    {
        serial = a.Serial,
        height = a.AvatarHeight,
        visual = Convert.ToHexStringLower(a.VisualParams),
        wearables = a.Wearables.Select(slot => Enumerable.Range(0, slot.Count)
            .Select(i => new[] { slot[i].ItemID.ToString(), slot[i].AssetID.ToString() })),
        attachments = a.GetAttachments().GroupBy(t => t.AttachPoint).OrderBy(g => g.Key)
            .Select(g => new { point = g.Key, items = g.Select(t => new[] { t.ItemID.ToString(), t.AssetID.ToString() }) })
    };

    public static int Run(string action, string pin, string path)
    {
        var reads = new List<object>();
        void Read(string name, string culture, params (string, string)[] fields)
        {
            CultureInfo.CurrentCulture = CultureInfo.GetCultureInfo(culture);
            var data = new AvatarData { Data = fields.ToDictionary(f => f.Item1, f => f.Item2) };
            reads.Add(new { name, culture, fields = fields.Select(f => new[] { f.Item1, f.Item2 }),
                appearance = Snapshot(data.ToAvatarAppearance()) });
        }

        Read("constructor", "en-US");
        Read("required body parts only", "en-US", ("Unknown", "ignored"));
        string[] legacy = ["Body", "Skin", "Hair", "Eyes", "Shirt", "Pants", "Shoes", "Socks",
            "Jacket", "Gloves", "UnderShirt", "UnderPants", "Skirt"];
        Read("legacy pairs", "en-US", legacy.SelectMany((key, i) => new[] {
            (key + "Item", Id(i + 1).ToString()), (key + "Asset", Id(i + 101).ToString())
        }).ToArray());
        Read("layer order and replacement", "en-US", ("Wearable 4:10", Pair(10)), ("Wearable 4:2", Pair(2)),
            ("Wearable 4", Pair(3)), ("ShirtItem", Id(2).ToString()), ("ShirtAsset", Id(20).ToString()),
            ("Wearable 4:-1", Pair(4)), ("Wearable 4:11", Pair(11)), ("Wearable 4:12", Pair(12)),
            ("Wearable 4:13", Pair(13)), ("Wearable 15:2", Pair(15)), ("Wearable 16:0", Pair(16)));
        Read("legacy and indexed zero tie", "en-US", ("Wearable 4", Pair(1)),
            ("Wearable 4:0", Pair(2)), ("Wearable 3:0", Pair(3)));
        // Different spellings of an index are distinct stored fields with equal sort keys.
        foreach (int count in new[] { 16, 17, 48 })
        {
            var rows = Enumerable.Range(0, count).Select(i =>
            {
                int type = 4 + (i % 12);
                string index = (type % 2).ToString("D" + (1 + i / 12));
                return ($"Wearable {type}:{index}", Pair(i + 1));
            }).ToArray();
            Read($"tied layers across {count} rows", "en-US", rows);
        }
        Read("attachments and parameters", "en-US", ("_ap_5", $"{Id(1)},bad,{Id(2)},{Id(3)},{Id(1)}"),
            ("_ap_8", $"{Id(2)},{Id(8)}"), ("_ap_bad", Id(9).ToString()), ("VisualParams", "-1,0,256, 19"),
            ("Serial", " -25 "));
        Read("UUID parsing", "en-US", ("BodyItem", " " + Id(41) + "suffix"), ("BodyAsset",Id(141).ToString()),
            ("Wearable 15", $"{Id(15)}suffix:{Id(115)}"), ("_ap_5", $"{Id(42)}suffix, {Id(43)} "));
        Read("comma in English", "en-US", ("AvatarHeight", "1,825"));
        Read("dot in German", "de-DE", ("AvatarHeight", "1.825"));
        Read("infinity symbol", "en-US", ("AvatarHeight", "∞"));
        Read("invalid abbreviated infinity", "en-US", ("AvatarHeight", "-inf"));
        Read("empty and extended slots", "en-US", ("Wearable 0", $"{UUID.Zero}:{Id(1)}"),
            ("Wearable 20", Pair(20)), ("_ap_5", $"{Id(5)},{Id(6)}"), ("_ap_05", Id(50).ToString()));
        Read("zero height", "en-US", ("AvatarHeight", "0"));
        Read("clamped high", "en-US", ("AvatarHeight", "99"));
        Read("clamped low", "en-US", ("AvatarHeight", "-1"));
        foreach (var field in new (string, string)[] { ("AvatarHeight","bad"), ("Serial","2147483648"),
            ("BodyItem",Id(1).ToString()), ("VisualParams","1,,2"), ("Wearable 4:bad",Pair(5)),
            ("Wearable -1:0",Pair(4)), ("Wearable 15","missing separator"), ("Wearable 4","bad:bad") })
            Read("malformed " + field.Item1, "en-US", field);

        // Independent writer inputs. No read observation is fed back into AvatarData.
        CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
        var writes = new List<object>();
        void Write(string name, AvatarAppearance a)
        {
            var data = new AvatarData(a);
            writes.Add(new { name, appearance = Snapshot(a), type = data.AvatarType, fields = data.Data,
                decoded = Snapshot(data.ToAvatarAppearance()) });
        }
        Write("constructor", new AvatarAppearance());
        var constructed = new AvatarAppearance { Serial = 23, VisualParams = [255,0,127,9] };
        constructed.SetSize(new Vector3(.45f, .6f, 1.825f));
        constructed.Wearables = Enumerable.Range(0, 17).Select(_ => new AvatarWearable()).ToArray();
        constructed.Wearables[4].Add(Id(4), Id(104));
        constructed.Wearables[4].Add(Id(5), Id(105));
        constructed.Wearables[15].Add(Id(15), Id(115));
        constructed.Wearables[16].Add(Id(16), Id(116));
        constructed.SetAttachment(7, Id(7), Id(107));
        constructed.SetAttachment(7 | 128, Id(8), Id(108));
        constructed.SetAttachment(3, Id(3), Id(103));
        Write("constructed", constructed);
        var json = JsonSerializer.Serialize(new { pin, reads, writes }, new JsonSerializerOptions { WriteIndented = true })
            .ReplaceLineEndings("\n") + "\n";
        if (action == "generate") File.WriteAllText(path, json);
        else if (action != "verify" || File.ReadAllText(path).ReplaceLineEndings("\n") != json)
            throw new InvalidOperationException("appearance observations differ");
        Console.WriteLine($"appearance: {reads.Count} reads and {writes.Count} independent writes match Pin8");
        return 0;
    }
}
