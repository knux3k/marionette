using UnityEngine;
public abstract class Character : MonoBehaviour { public float health = 100f; }
public class Enemy : Character
{
    void FixedUpdate()
    {
        var t = System.DateTime.Now;
        GameObject player = GameObject.Find("Player");
    }
}
public static class MathUtil { public static float Square(float x) { return x * x; } }
