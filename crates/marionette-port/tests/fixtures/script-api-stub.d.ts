// Minimal stand-in for the script API typings (decision 0019) so the porting tests can
// type-check generated skeletons without the script crate. Only what skeletons use.
interface Entity {
  readonly id: number;
  get(name: string): any;
  set(name: string, value: unknown): void;
  has(name: string): boolean;
  state: Record<string, unknown>;
}
