// The pet page stays plain TypeScript to keep idle memory low (architecture 7).
// V1-0 skeleton: the pet shell, states and art are slice V1-1.
import { scaffoldNotice } from '../shared/build';

const root = document.getElementById('pet');
if (root) {
  root.textContent = scaffoldNotice('Pet');
}
