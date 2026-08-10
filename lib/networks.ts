/**
 * Arranging saved networks by the place they belong to.
 *
 * One rule, used by every view that shows the list. Two views deriving their
 * own grouping is how the sidebar and the settings panel end up disagreeing
 * about which networks are at the same site.
 */

import type { Location, NetworkProfile } from "./types";

export interface NetworkGroup {
  /** `null` is the bucket for networks not filed under any location. */
  location: Location | null;
  networks: NetworkProfile[];
}

/**
 * Groups networks under their locations, keeping the order of both inputs.
 *
 * Locations arrive already sorted from the engine, and networks keep their
 * order within a group, so nothing here reorders anything the user has not
 * asked to be reordered. The unassigned bucket is always last.
 *
 * A `locationId` that matches no location falls into the unassigned bucket
 * rather than being dropped. The engine heals these on load, so it should not
 * happen — but a network vanishing from every list is a worse failure than a
 * redundant check.
 */
export function groupByLocation(
  networks: NetworkProfile[],
  locations: Location[],
  options?: {
    /** Keep locations with no networks in them. The management panel wants
     *  these so an empty location can still be renamed or deleted; a dropdown
     *  does not, because they would just be dead headers. */
    includeEmpty?: boolean;
  },
): NetworkGroup[] {
  const byLocation = new Map<string, NetworkProfile[]>(
    locations.map((location) => [location.id, []]),
  );
  const unassigned: NetworkProfile[] = [];

  for (const network of networks) {
    const bucket = network.locationId ? byLocation.get(network.locationId) : undefined;
    if (bucket) bucket.push(network);
    else unassigned.push(network);
  }

  const groups: NetworkGroup[] = [];
  for (const location of locations) {
    const members = byLocation.get(location.id) ?? [];
    if (members.length > 0 || options?.includeEmpty) {
      groups.push({ location, networks: members });
    }
  }
  if (unassigned.length > 0) groups.push({ location: null, networks: unassigned });

  return groups;
}
