**The Render view no longer falls apart when the camera turns**

Turning or moving the camera on the Render side no longer throws the scene away and builds it again in pieces floating over the sea: a map within the view distance is loaded whole once and stays, and on a larger map the part around where the camera went is built in the background while the land already there stays shown, taking its place once whole.
