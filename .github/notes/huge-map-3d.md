**A closed 3D view no longer slows the map down**

On a huge map, closing the 3D view left it building the tiles of the whole map in the background, on nearly every core, for a minute or more: the 2D map stayed slow after it. A 3D view out of sight now stops building at once, and after 20 seconds gives back the memory it holds on the graphics card; opening it again builds what it shows. Its building threads are also fewer on processors with many cores, and on Windows they run below the interface's priority.
