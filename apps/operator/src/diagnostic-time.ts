// Anchor to request START, so response/network delay can only shorten presentation validity.
export function diagnosticDeadline(requestStarted: number, validForNs: string): number {
  return requestStarted + Number(BigInt(validForNs)) / 1_000_000;
}
export function diagnosticFresh(
  requestStarted: number,
  validForNs: string,
  now: number,
  queryFresh: boolean,
): boolean {
  return (
    queryFresh && now >= requestStarted && now < diagnosticDeadline(requestStarted, validForNs)
  );
}
