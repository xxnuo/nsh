import { expect, open, test } from "nsh/test";

test("bounds read materialized rows without scrolling them into the viewport", async () => {
  const app = await open("./Geometry.tsx");
  const list = app.bounds("id:list");
  const row = app.bounds("id:last");
  if (!list || !row) throw new Error("the scroll view and its materialized row need bounds");
  expect(app.shows("id:last")).toBe(true);
  await app.waitFor("id:last");
  expect(row.origin.y).toBeGreaterThan(list.origin.y + list.size.height);
  expect(app.bounds("id:last")).toEqual(row);
  expect(app.bounds("id:missing")).toBeNull();
  expect(app.bounds("id:virtual-0")).not.toBeNull();
  expect(app.shows("id:virtual-99")).toBe(false);
  expect(app.bounds("id:virtual-99")).toBeNull();
  expect(app.shows("id:virtual-99")).toBe(false);
  expect(() => app.bounds("text:/row$/")).toThrow(/matches 2 views/);
  expect(() => app.bounds("unknown:row")).toThrow(/unknown selector prefix/);
  expect(app.bounds("text:/row$/ >> nth=1")).toEqual(row);
  const viewport = app.viewport();
  expect(viewport.width).toBeGreaterThan(0);
  expect(viewport.height).toBeGreaterThan(0);
  await app.scroll("id:list", 400);
  const moved = app.bounds("id:last");
  if (!moved) throw new Error("the row must remain materialized after scrolling");
  expect(moved.origin.y).toBeLessThan(row.origin.y);
});

test("geometry belongs to each App and follows only that App's resize", async () => {
  const first = await open("./Geometry.tsx", { width: 320, height: 480 });
  const second = await open("./Geometry.tsx", { width: 600, height: 800 });
  expect(first.viewport()).toEqual({ width: 320, height: 480 });
  expect(second.viewport()).toEqual({ width: 600, height: 800 });
  expect(first.bounds("id:list")?.size.width).toBe(320);
  expect(second.bounds("id:list")?.size.width).toBe(600);
  await first.setViewport(390, 600);
  expect(first.viewport()).toEqual({ width: 390, height: 600 });
  expect(first.bounds("id:list")?.size.width).toBe(390);
  expect(second.viewport()).toEqual({ width: 600, height: 800 });
  expect(second.bounds("id:list")?.size.width).toBe(600);
});
