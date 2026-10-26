import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Combo, Select } from "./Select";

afterEach(cleanup);

const fruit = [
  { value: "apple", label: "Apple" },
  { value: "pear", label: "Pear" },
  { value: "plum", label: "Plum" },
];

describe("Select", () => {
  it("shows the current label and opens a listbox on click", () => {
    render(<Select value="pear" options={fruit} onChange={() => {}} />);
    const trigger = screen.getByRole("combobox");
    expect(trigger.textContent).toBe("Pear");
    expect(screen.queryByRole("listbox")).toBeNull();
    fireEvent.click(trigger);
    expect(screen.getAllByRole("option")).toHaveLength(3);
    expect(screen.getByRole("option", { selected: true }).textContent).toBe("Pear");
  });

  it("picks an option with the mouse", () => {
    const onChange = vi.fn();
    render(<Select value="apple" options={fruit} onChange={onChange} />);
    fireEvent.click(screen.getByRole("combobox"));
    fireEvent.mouseDown(screen.getByRole("option", { name: "Plum" }));
    expect(onChange).toHaveBeenCalledWith("plum");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("moves with the arrow keys and picks with Enter", () => {
    const onChange = vi.fn();
    render(<Select value="apple" options={fruit} onChange={onChange} />);
    const trigger = screen.getByRole("combobox");
    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "ArrowDown" });
    fireEvent.keyDown(trigger, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith("plum");
  });

  it("closes on Escape without letting the key reach the window", () => {
    const outer = vi.fn();
    render(
      <div onKeyDown={outer}>
        <Select value="apple" options={fruit} onChange={() => {}} />
      </div>,
    );
    const trigger = screen.getByRole("combobox");
    fireEvent.click(trigger);
    fireEvent.keyDown(trigger, { key: "Escape" });
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(outer).not.toHaveBeenCalled();
  });

  it("doesn't report a change when the current option is picked again", () => {
    const onChange = vi.fn();
    render(<Select value="apple" options={fruit} onChange={onChange} />);
    fireEvent.click(screen.getByRole("combobox"));
    fireEvent.mouseDown(screen.getByRole("option", { name: "Apple" }));
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("Combo", () => {
  it("narrows suggestions to what was typed", () => {
    const models = ["qwen3:4b", "llama3.2", "qwen2.5-coder"];
    const { rerender } = render(<Combo value="" suggestions={models} onChange={() => {}} />);
    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "qwen" } });
    rerender(<Combo value="qwen" suggestions={models} onChange={() => {}} />);
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["qwen3:4b", "qwen2.5-coder"]);
  });
});
