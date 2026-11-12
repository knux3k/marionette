using UnityEditor;
[CustomEditor(typeof(PlayerController))]
public class PlayerEditor : Editor { public override void OnInspectorGUI() { DrawDefaultInspector(); } }
