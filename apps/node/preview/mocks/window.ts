export class LogicalSize {
  constructor(public width: number, public height: number) {}
}
export function getCurrentWindow() {
  return {
    setDecorations: async (_on: boolean) => {},
    setSize: async (_s: LogicalSize) => {},
  };
}
