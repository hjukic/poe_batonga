// A button that acts on its second click: the first click arms it ("Click again to reset"); it
// disarms by itself after a few seconds, or when told (the menu closes). A click that comes
// too soon after arming is the second half of a double click, not a decision: it is ignored
// (the button stays armed). DOM-free: scripts/two-step.test.mjs.

export interface TwoStep {
  readonly armed: boolean;
  /** True when this click is the one to act on; the first click only arms. */
  click(): boolean;
  disarm(): void;
}
export interface TwoStepOptions {
  /** A click within this many ms of arming is ignored (a double click's second click). */
  guard?: number;
  now?: () => number;
}

export function twoStep(ms: number, onChange: (armed: boolean) => void, { guard = 700, now = () => Date.now() }: TwoStepOptions = {}): TwoStep {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let armedAt = 0;
  const disarm = () => {
    if (timer === null) return;
    clearTimeout(timer);
    timer = null;
    onChange(false);
  };
  return {
    get armed() { return timer !== null; },
    click() {
      if (timer !== null) {
        if (now() - armedAt < guard) return false;
        disarm();
        return true;
      }
      armedAt = now();
      timer = setTimeout(disarm, ms);
      onChange(true);
      return false;
    },
    disarm,
  };
}

/** The button's click handler: acts only on the confirming click. */
export const confirmedClick = (step: TwoStep, act: () => void) => () => {
  if (step.click()) act();
};
