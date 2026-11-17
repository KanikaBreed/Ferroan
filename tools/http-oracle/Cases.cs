// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

// Inputs for ServerUtils. The outputs are whatever that C# returns.

using System.Text;
using OpenMetaverse;

namespace Ferroan.HttpOracle;

static class Cases
{
    internal static List<Case> Build()
    {
        List<Case> cases = [];

        void QueryParse(string id, string raw) =>
            cases.Add(new Case(id, "query-parse", raw, null));

        void QueryBuild(string id, Dictionary<string, object> value) =>
            cases.Add(new Case(id, "query-build", null, value));

        void XmlBuild(string id, Dictionary<string, object> value) =>
            cases.Add(new Case(id, "xml-build", null, value));

        void XmlParse(string id, string raw) =>
            cases.Add(new Case(id, "xml-parse", raw, null));

        QueryParse("query-empty", "");
        QueryParse("query-one", "a=1");
        QueryParse("query-two", "a=1&b=2");
        QueryParse("query-space-plus", "a=hello+world");
        QueryParse("query-space-pct", "a=hello%20world");
        QueryParse("query-list", "q[]=a&q[]=b");
        QueryParse("query-list-empty", "q[]=&q[]=b");
        QueryParse("query-dup", "a=1&a=2");
        QueryParse("query-list-then-scalar", "a[]=1&a=2");
        QueryParse("query-scalar-then-list", "a=1&a[]=2");
        QueryParse("query-blank-name", "&=x&a=1");
        QueryParse("query-empty-value", "a=&b");
        QueryParse("query-raw-whitespace", "a=   ");
        QueryParse("query-plus-only", "a=+");
        QueryParse("query-pct-space", "a=%20");
        QueryParse("query-equals-in-value", "a=b=c");
        QueryParse("query-encoded-list-name", "a%5B%5D=x");
        QueryParse("query-empty-list-name", "[]=x");
        QueryParse("query-unicode", "n=%C3%A9");
        QueryParse("query-u-escape", "n=%u00e9");
        QueryParse("query-bad-pct", "a=%zz&b=%2&c=%2G");
        QueryParse("query-encoded-amp", "a=x%26y");
        QueryParse("query-key-space", "a%20b=c");
        QueryParse("query-trailing-amp", "a=1&");

        QueryBuild("query-build-empty-value", Map(("a", "")));
        QueryBuild("query-build-two", Map(("a", "1"), ("b", "2")));
        QueryBuild("query-build-space", Map(("a", "hello world")));
        QueryBuild("query-build-list", Map(("q", new List<string> { "a", "b" })));
        QueryBuild("query-build-list-empty", Map(("q", new List<string> { "", "b" })));
        QueryBuild("query-build-bool-int", Map(("ok", true), ("n", 1)));
        QueryBuild("query-build-sweep", Map(("s", AsciiSweep())));
        QueryBuild("query-build-json", Map(("blob", Map(("id", "one"), ("name", "Ada")))));
        QueryBuild("query-build-json-again", Map(("blob", Map(("id", "two")))));
        QueryBuild("query-build-json-uuid", Map(("blob", Map(
            ("id", new UUID("00112233-4455-6677-8899-aabbccddeeff")),
            ("tags", new List<string> { "a", "b" })))));
        QueryBuild("query-build-null", Map(("a", "1"), ("gone", null)));

        XmlBuild("xml-empty", Map());
        XmlBuild("xml-flat", Map(("Success", "true"), ("Message", "hello")));
        XmlBuild("xml-nested", Map(("friend0", Map(("Principal", "abc"), ("Flags", "1")))));
        XmlBuild("xml-null-skip", Map(("keep", "yes"), ("skip", null), ("after", "1")));
        XmlBuild("xml-escape", Map(("text", "a<b>&\"'")));
        XmlBuild("xml-names", Map(
            ("a b", "1"),
            ("a:b", "2"),
            ("1abc", "3"),
            ("xml", "4"),
            ("_x0020_", "5"),
            ("ok", "6"),
            ("café", "7")));
        XmlBuild("xml-empty-string", Map(("a", "")));
        XmlBuild("xml-unicode", Map(("city", "Malmö")));
        XmlBuild("xml-bool-int", Map(("ok", true), ("n", 1)));
        XmlBuild("xml-two-lists", Map(
            ("one", Map(("a", "1"))),
            ("two", Map(("b", "2")))));
        XmlBuild("xml-empty-key", Map(("", "x")));

        XmlParse("xml-garbage", "not xml");
        XmlParse("xml-no-root", "<other>a</other>");
        XmlParse("xml-comment", "<!-- note --><ServerResponse><a>1</a></ServerResponse>");
        XmlParse("xml-pretty", """
            <?xml version="1.0"?>
            <ServerResponse>
              <a>1</a>
              <b>2</b>
            </ServerResponse>
            """);
        XmlParse("xml-bad-child", "<ServerResponse><a>1<b>2</b></a></ServerResponse>");
        XmlParse("xml-other-type", "<ServerResponse><a type=\"Foo\">bar</a></ServerResponse>");
        XmlParse("xml-empty-list", "<ServerResponse><Friends type=\"List\" /></ServerResponse>");
        XmlParse("xml-char-ref", "<ServerResponse><a>&#65;&#x42;</a></ServerResponse>");
        XmlParse("xml-empty-input", "");
        XmlParse("xml-sibling-after-list", "<ServerResponse><group type=\"List\"><a>1</a></group><after>2</after></ServerResponse>");
        XmlParse("xml-cdata", "<ServerResponse><a><![CDATA[x<y]]></a></ServerResponse>");
        XmlParse("xml-space-text", "<ServerResponse><a> a </a></ServerResponse>");

        return cases;
    }

    internal static Dictionary<string, object> Map(params (string Key, object Value)[] pairs)
    {
        Dictionary<string, object> map = new(pairs.Length);
        foreach ((string key, object value) in pairs)
            map[key] = value;
        return map;
    }

    static string AsciiSweep()
    {
        StringBuilder builder = new(128);
        for (int i = 0; i < 128; i++)
            builder.Append((char)i);
        return builder.ToString();
    }
}

sealed class Case(string id, string kind, string raw, Dictionary<string, object> value)
{
    internal string Id { get; } = id;
    internal string Kind { get; } = kind;
    internal string Raw { get; } = raw;
    internal Dictionary<string, object> Value { get; } = value;
}
