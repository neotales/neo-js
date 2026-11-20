import {
  accountGroupIdsSync,
  currentUserSync,
  homeDirSync,
  processIdsSync,
} from "neo:os/user";
import type { ProcessIds, UserAccount, UserErrorCode } from "neo:os/user";

const account: UserAccount = currentUserSync();
const ids: ProcessIds = processIdsSync();
const home: string = homeDirSync();
const primary: number = account.primaryGid;
const effective: number = ids.effectiveUid;
const groupIds: number[] = accountGroupIdsSync();
const unsupported: UserErrorCode = "USER_UNSUPPORTED";

// @ts-expect-error The account record has no effective credentials.
account.effectiveUid;
// @ts-expect-error These APIs do not query arbitrary accounts.
currentUserSync(123);
// @ts-expect-error This first increment does not expose a temp directory.
import { tempDirSync } from "neo:os/user";
// @ts-expect-error The functions do not return promises.
const asyncHome: Promise<string> = homeDirSync();

void [home, primary, effective, groupIds, unsupported, tempDirSync, asyncHome];
