// Vite's inline worker import (the camera page's pose worker): the module's code is
// bundled into the page and started from a blob, so it inherits the page's content
// policy.
declare module '*?worker&inline' {
  const WorkerFactory: new () => Worker;
  export default WorkerFactory;
}
