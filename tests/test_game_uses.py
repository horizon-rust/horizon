from pathlib import Path
import tempfile
import unittest

from tools import game_uses

GAME = '''
struct Game<'a> {
    mobs: Vec<u32>,
    world: World,
    next_entity: u32,
    ui: Ui,
    log: Vec<String>,
}

impl Game<'_> {
    /// self.ui in a comment is not a use
    pub(super) fn tick(&mut self, n: u32) -> u32 {
        let text = "self.log.push(1)";
        let c = 'x';
        for m in self.mobs.iter() {
            self.world.random.next_float();
        }
        self.mobs.push(n);
        self.next_entity += 1;
        let r = &mut self.world;
        let d = self.ui.difficulty + self.ui.level;
        self.log[0] = text.to_string();
        self.helper(&mut self.mobs, 1);
        self.spawn_item(n);
        self.send_packet(Packet::Ping);
        Self::assoc(3);
        MobEvent::Spawn { id: Foo::new() };
        free_fn(self);
        0
    }
    fn helper(&self, a: &mut Vec<u32>, b: u32) {}
    fn spawn_item(&mut self, n: u32) {}
    fn send_packet(&mut self, p: Packet) {}
    fn by_value(self) {}
    fn assoc(n: u32) {}
    fn takes_impl(&mut self, f: impl FnOnce()) {}
    fn takes_result(&self, result: u32) {}
    fn native_tick(&mut self) {}
    #[inline(always)]
    fn forced(&self) {}
    #[cfg(test)]
    fn only_test(&self) {}
    fn gives_impl(&self) -> impl Iterator<Item = u32> { std::iter::empty() }
    fn ok_ref_impl(&self, f: &impl Fn()) {}
    unsafe fn risky(&self) {}
}

impl Trait for Game<'_> {
    fn on_trait(&mut self) {}
}

impl Other {
    fn tick(&self) { self.mobs.len(); }
}
'''


class GameUsesTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        root = Path(cls.tmp.name).resolve()
        (root / "game.rs").write_text(GAME)
        cls.index = game_uses.Index(root, root)
        cls.src = cls.index.sources[root / "game.rs"]

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def uses(self, name="tick"):
        (_, src, start, end, fn), *_ = self.index.find(f"game.rs::{name}")  # the Game one first
        return game_uses.analyse(self.index, src, start, end, fn)

    def test_game_fields_are_read_from_the_struct(self):
        self.assertEqual(self.index.fields, ["mobs", "world", "next_entity", "ui", "log"])

    def test_fields_split_into_reads_and_writes(self):
        fields = self.uses().fields
        # mobs: `.iter()` reads; `.push(n)` and `&mut self.mobs` write
        self.assertEqual((fields["mobs"]["read"], fields["mobs"]["write"]), (1, 2))
        self.assertEqual(fields["next_entity"], {"read": 0, "write": 1, "lines": fields["next_entity"]["lines"]})
        self.assertEqual(fields["ui"]["read"], 2)
        self.assertEqual(fields["ui"]["write"], 0)
        self.assertEqual(fields["log"]["write"], 1)
        self.assertEqual(fields["world"]["write"], 1)  # `&mut self.world`
        self.assertEqual(fields["world"]["read"], 1)  # `.random.next_float()`

    def test_comments_and_strings_are_not_uses(self):
        self.assertNotIn("push(1)", " ".join(self.uses().methods))
        self.assertEqual(self.uses().fields["log"]["write"], 1)

    def test_called_methods_and_whole_self(self):
        uses = self.uses()
        self.assertEqual(set(uses.methods), {"helper", "spawn_item", "send_packet"})
        self.assertEqual(set(uses.assoc), {"assoc()"})
        self.assertEqual(len(uses.whole_self), 1)
        self.assertIn("free_fn(self)", uses.whole_self[0])

    def test_markers_keep_their_lines(self):
        markers = {(m["category"], m["match"]) for m in self.uses().markers}
        self.assertIn(("random", "random"), markers)
        self.assertIn(("random", "next_float"), markers)
        self.assertIn(("entity id", "next_entity"), markers)
        self.assertIn(("spawn", "spawn_item"), markers)
        self.assertIn(("send", "send_packet("), markers)
        lines = [m["line"] for m in self.uses().markers]
        self.assertEqual(lines, sorted(lines))

    def test_named_items_fold_enum_members(self):
        names = set(self.uses().names)
        self.assertIn("MobEvent::Spawn", names)
        self.assertIn("Foo::new", names)
        self.assertIn("free_fn", names)

    def candidate(self, name):
        (found,) = [f for f in self.index.methods[name]]
        return found.eligible()

    def test_point_eligibility_follows_the_shape_rule(self):
        self.assertTrue(self.candidate("tick")[0])
        self.assertTrue(self.candidate("ok_ref_impl")[0])  # `&impl Fn()` is a reference, not `impl Trait`
        for name, word in [("by_value", "receiver"), ("assoc", "receiver"), ("takes_impl", "impl Trait"),
                           ("takes_result", "result"), ("native_tick", "native_"), ("forced", "inline(always)"),
                           ("only_test", "cfg"), ("gives_impl", "returns impl"), ("risky", "unsafe")]:
            ok, why = self.candidate(name)
            self.assertFalse(ok, name)
            self.assertIn(word, why, name)

    def test_trait_impls_and_other_types_are_kept_apart(self):
        self.assertNotIn("on_trait", self.index.methods)
        self.assertIn("on_trait", self.index.trait_methods)
        self.assertEqual(len(self.index.methods["tick"]), 1)  # not `impl Other`'s `tick`
        info = game_uses.method_info(self.index, "on_trait")
        self.assertFalse(info["candidate"])

    def test_line_range_spec(self):
        first = self.src.text[:self.src.text.index("self.world.random")].count("\n") + 1
        (_, src, start, end, fn), = self.index.find(f"game.rs:{first}-{first}")
        self.assertIsNone(fn)
        uses = game_uses.analyse(self.index, src, start, end, None)
        self.assertEqual(set(uses.fields), {"world"})


class BlankTest(unittest.TestCase):
    def test_keeps_offsets_and_lifetimes(self):
        text = "fn f<'a>(x: &'a str) { let c = '}'; /* { */ let s = r#\"}\"#; // }\n}\n"
        out = game_uses.blank(text)
        self.assertEqual(len(out), len(text))
        self.assertEqual(out.count("{"), 1)
        self.assertEqual(out.count("}"), 1)
        self.assertIn("'a", out)


if __name__ == "__main__":
    unittest.main()
