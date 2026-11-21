// The intent that puts an uploaded image on a level (ADR 0014, contract issue #51). It
// only names the asset; the server fills in the size and address from its own record.
import { create } from '@bufbuild/protobuf';
import { SetLevelBackgroundIntentSchema } from '@tomeandtile/contracts';
import type { IntentKind } from '../net';
import type { MapTarget } from '../net';

export function buildSetBackgroundKind(target: MapTarget, assetId: string): IntentKind {
	return {
		case: 'setLevelBackground',
		value: create(SetLevelBackgroundIntentSchema, {
			sceneId: target.sceneId,
			levelId: target.levelId,
			assetId
		})
	};
}
